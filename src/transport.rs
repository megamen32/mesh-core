//! GrepMesh peer transport, shared without retaining request credentials.
//! A consumer supplies its already-scoped connection pool and immutable limits.
use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{error::Error as StdError, time::Duration};
use tokio::time::timeout;

pub struct PeerTransport<'a> {
    pub client: &'a reqwest::Client,
    pub peer_auth_token: Option<&'a str>,
    pub max_response_bytes: usize,
    pub peer_timeout_ms: u64,
}
impl PeerTransport<'_> {
    pub async fn call_remote_direct(&self, url: &str, tool: &str, arguments: Value) -> Result<Value> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        });
        let max_response_bytes = self.max_response_bytes;
        let value = timeout(
            Duration::from_millis(self.peer_timeout_ms),
            async {
                let mut request = self
                    .client
                    .post(url)
                    .header("Accept", "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", "2026-07-28")
                    .header("Mcp-Method", "tools/call")
                    .header("Mcp-Name", tool);
                if let Some(token) = self.peer_auth_token.as_deref() {
                    request = request.bearer_auth(token);
                }
                let resp = request
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|error| anyhow!(Self::redacted_outbound_error(&error)))?;
                if !resp.status().is_success() {
                    return Err(anyhow!("remote HTTP status {}", resp.status()));
                }
                let mut stream = resp.bytes_stream();
                let mut body = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.context("read remote response")?;
                    if max_response_bytes != 0
                        && body.len().saturating_add(chunk.len()) > max_response_bytes
                    {
                        return Err(anyhow!(
                            "remote response exceeds max_response_bytes ({max_response_bytes})"
                        ));
                    }
                    body.extend_from_slice(&chunk);
                }
                serde_json::from_slice::<Value>(&body).context("parse remote response")
            },
        )
        .await??;
        if let Some(err) = value.get("error") {
            return Err(anyhow!("remote error: {}", err));
        }
        Ok(value.get("result").cloned().unwrap_or(value))
    }

    pub async fn call_remote_via_gptadmin_proxy(
        &self,
        proxy_url: &str,
        peer_url: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let proxy = validated_loopback_proxy_url(proxy_url)?;
        let peer = reqwest::Url::parse(peer_url).context("parse peer URL for GPTAdmin fallback")?;
        if peer.scheme() != "http" {
            return Err(anyhow!(
                "GPTAdmin fallback supports only http peer URLs; got {}",
                peer.scheme()
            ));
        }
        let host = peer
            .host_str()
            .ok_or_else(|| anyhow!("peer URL has no host"))?;
        let port = peer
            .port_or_known_default()
            .ok_or_else(|| anyhow!("peer URL has no port"))?;
        let target = format!("{host}:{port}");
        let proxy_host = proxy
            .host_str()
            .ok_or_else(|| anyhow!("GPTAdmin proxy URL has no host"))?;
        let proxy_port = proxy.port_or_known_default().unwrap_or(80);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        })
        .to_string();
        let path = match peer.query() {
            Some(query) => format!("{}?{query}", peer.path()),
            None => peer.path().to_string(),
        };
        timeout(
            Duration::from_millis(self.peer_timeout_ms),
            async {
                let mut stream = TcpStream::connect((proxy_host, proxy_port))
                    .await
                    .context("connect to GPTAdmin loopback proxy")?;
                let connect = format!(
                    "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nConnection: keep-alive\r\n\r\n"
                );
                stream
                    .write_all(connect.as_bytes())
                    .await
                    .context("write proxy CONNECT")?;
                let connect_response = read_http_headers(&mut stream).await?;
                if !connect_response.starts_with("HTTP/1.1 200")
                    && !connect_response.starts_with("HTTP/1.0 200")
                {
                    return Err(anyhow!("GPTAdmin proxy CONNECT was rejected"));
                }
                let mut request = format!(
                    "POST {path} HTTP/1.1\r\nHost: {target}\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\nMcp-Name: {tool}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    payload.len()
                );
                if let Some(token) = self.peer_auth_token.as_deref() {
                    request.push_str(&format!("Authorization: Bearer {token}\r\n"));
                }
                request.push_str("\r\n");
                stream
                    .write_all(request.as_bytes())
                    .await
                    .context("write tunneled request")?;
                stream
                    .write_all(payload.as_bytes())
                    .await
                    .context("write tunneled payload")?;
                let headers = read_http_headers(&mut stream).await?;
                if !headers.starts_with("HTTP/1.1 2") && !headers.starts_with("HTTP/1.0 2") {
                    return Err(anyhow!("remote HTTP status via GPTAdmin fallback"));
                }
                let max_response_bytes = self.max_response_bytes;
                let mut body = Vec::new();
                let mut chunk = [0; 8 * 1024];
                loop {
                    let read = stream
                        .read(&mut chunk)
                        .await
                        .context("read tunneled response")?;
                    if read == 0 {
                        break;
                    }
                    if max_response_bytes != 0
                        && body.len().saturating_add(read) > max_response_bytes
                    {
                        return Err(anyhow!(
                            "remote response exceeds max_response_bytes ({max_response_bytes})"
                        ));
                    }
                    body.extend_from_slice(&chunk[..read]);
                }
                let value =
                    serde_json::from_slice::<Value>(&body).context("parse tunneled remote response")?;
                if let Some(err) = value.get("error") {
                    return Err(anyhow!("remote error: {}", err));
                }
                Ok(value.get("result").cloned().unwrap_or(value))
            },
        )
        .await
        .context("GPTAdmin fallback timeout")?
    }

    pub fn redacted_outbound_error(error: &reqwest::Error) -> String {
        let phase = if error.is_timeout() {
            "timeout"
        } else if error.is_connect() {
            "connect"
        } else if error.is_body() {
            "body"
        } else if error.is_request() {
            "request"
        } else {
            "transport"
        };
        let mut source = error.source();
        while let Some(cause) = source {
            if let Some(io_error) = cause.downcast_ref::<std::io::Error>() {
                let mut summary = format!(
                    "send remote request: {phase}; io_kind={:?}",
                    io_error.kind()
                );
                if let Some(errno) = io_error.raw_os_error() {
                    summary.push_str(&format!("; errno={errno}"));
                }
                return summary;
            }
            source = cause.source();
        }
        format!("send remote request: {phase}")
    }

}

pub fn is_direct_connect_error(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .starts_with("send remote request: connect")
}

pub fn validated_loopback_proxy_url(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value.trim()).context("parse GPTAdmin proxy URL")?;
    let loopback = matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("::1") | Some("[::1]")
    );
    if url.scheme() != "http"
        || !loopback
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(anyhow!(
            "GPTAdmin fallback proxy must be a credential-free http loopback URL with no path"
        ));
    }
    Ok(url)
}

async fn read_http_headers(stream: &mut tokio::net::TcpStream) -> Result<String> {
    use tokio::io::AsyncReadExt;
    let mut headers = Vec::new();
    while headers.len() < 32 * 1024 {
        let mut byte = [0u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .context("read HTTP headers")?;
        headers.push(byte[0]);
        if headers.ends_with(b"\r\n\r\n") {
            return String::from_utf8(headers).context("decode HTTP headers");
        }
    }
    Err(anyhow!("HTTP headers exceed 32768 bytes"))
}
