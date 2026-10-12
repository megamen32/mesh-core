//! One local Rust process for non-Rust consumers. No stdio workers.
//! The authenticated product edge supplies already-authorized topology per call.
//! This endpoint retains no profiles, tokens, session data or request IDs.
use anyhow::{anyhow, Context, Result};
use mesh_core::{request::{normalize_request, HostsInput}, topology::Topology, topology_cache::TopologySnapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt}, path::PathBuf, sync::Arc, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{UnixListener, UnixStream}, sync::Semaphore, time::timeout};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Normalize {
    local_host_id: String,
    known_hosts: Vec<String>,
    hosts: Option<HostsInput>,
    request_id: Option<String>,
    origin_host: Option<String>,
    hop_count: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot { snapshot: TopologySnapshot, now_ms: u64 }

struct OwnedSocket { path: PathBuf, dev: u64, inode: u64, uid: u32 }
fn allowed_peer(owner: u32, peer: u32) -> bool { peer == owner || peer == 0 }
impl Drop for OwnedSocket {
    fn drop(&mut self) {
        if let Ok(metadata) = std::fs::symlink_metadata(&self.path) {
            if metadata.file_type().is_socket() && metadata.dev() == self.dev && metadata.ino() == self.inode && metadata.uid() == self.uid {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

fn dispatch(path: &str, body: &[u8]) -> Result<Value> {
    match path {
        "/v1/normalize" => {
            let input: Normalize = serde_json::from_slice(body)?;
            if input.local_host_id.is_empty() || input.local_host_id.len() > 512 || input.known_hosts.len() > 100 {
                return Err(anyhow!("invalid bounded topology"));
            }
            if input.known_hosts.iter().any(|s| s.is_empty() || s.len() > 512) {
                return Err(anyhow!("invalid host identity"));
            }
            serde_json::to_value(normalize_request(&input.local_host_id, input.known_hosts, input.hosts, input.request_id, input.origin_host, input.hop_count)?).map_err(Into::into)
        }
        "/v1/topology" => {
            let input: Snapshot = serde_json::from_slice(body)?;
            if input.snapshot.peers.len() > 100 { return Err(anyhow!("too many peers")); }
            let topology = Topology::from_snapshot(input.snapshot, input.now_ms)?;
            Ok(json!({"status":topology.status(), "local_host_id":topology.local_host_id, "peers":topology.peers}))
        }
        _ => Err(anyhow!("unsupported core operation")),
    }
}

async fn handle(mut stream: UnixStream, uid: u32) -> Result<()> {
    // The root-owned GPTAdmin service already has privileged access to this
    // private directory/socket. Other users remain outside the product edge.
    let peer_uid = stream.peer_cred()?.uid();
    if !allowed_peer(uid, peer_uid) { return Err(anyhow!("foreign Unix peer")); }
    let mut header = Vec::new();
    loop {
        if header.len() >= 8192 { return Err(anyhow!("headers too large")); }
        let mut byte = [0]; stream.read_exact(&mut byte).await?; header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") { break; }
    }
    let header = std::str::from_utf8(&header)?;
    let mut lines = header.split("\r\n");
    let first = lines.next().ok_or_else(|| anyhow!("missing request line"))?;
    let mut parts = first.split_whitespace();
    if parts.next() != Some("POST") { return Err(anyhow!("POST required")); }
    let path = parts.next().ok_or_else(|| anyhow!("missing operation"))?;
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() { return Err(anyhow!("invalid request line")); }
    let mut length = None;
    let mut content_type = None;
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(':').ok_or_else(|| anyhow!("invalid header"))?;
        match name.to_ascii_lowercase().as_str() {
            "content-length" => {
                if length.is_some() { return Err(anyhow!("duplicate content length")); }
                length = Some(value.trim().parse::<usize>()?);
            }
            "content-type" => {
                if content_type.is_some() { return Err(anyhow!("duplicate content type")); }
                content_type = Some(value.trim());
            }
            "transfer-encoding" => return Err(anyhow!("chunked request unsupported")),
            _ => (),
        }
    }
    let length = length.ok_or_else(|| anyhow!("content length required"))?;
    if length > 128 * 1024 || content_type != Some("application/json") { return Err(anyhow!("invalid bounded JSON request")); }
    let mut body = vec![0; length]; stream.read_exact(&mut body).await?;
    // Never include input, credentials, URLs or parser messages in a public error.
    let (status, value) = match dispatch(path, &body) {
        Ok(value) => ("200 OK", value),
        Err(_) => ("400 Bad Request", json!({"error":"core_request_rejected"})),
    };
    let body = serde_json::to_vec(&value)?;
    if body.len() > 1024 * 1024 { return Err(anyhow!("response too large")); }
    let header = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(header.as_bytes()).await?; stream.write_all(&body).await?;
    stream.shutdown().await?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--socket") { return Err(anyhow!("usage: mesh-core-service --socket PRIVATE_PATH")); }
    let socket = PathBuf::from(args.next().ok_or_else(|| anyhow!("socket path required"))?);
    if args.next().is_some() || !socket.is_absolute() { return Err(anyhow!("exact absolute socket required")); }
    let parent = socket.parent().ok_or_else(|| anyhow!("private parent required"))?;
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 || parent.canonicalize()? != parent {
        return Err(anyhow!("canonical private directory required"));
    }
    // Refuse any existing socket/file; never adopt or unlink another generation.
    if std::fs::symlink_metadata(&socket).is_ok() { return Err(anyhow!("socket already exists")); }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let listener = UnixListener::bind(&socket).context("bind private core socket")?;
        let owned = std::fs::symlink_metadata(&socket)?;
        let _generation = OwnedSocket { path: socket.clone(), dev: owned.dev(), inode: owned.ino(), uid: owned.uid() };
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        if owned.uid() != metadata.uid() { return Err(anyhow!("private directory owner mismatch")); }
        let slots = Arc::new(Semaphore::new(8));
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let serving = async {
            loop {
                let permit = slots.clone().acquire_owned().await?;
                let (stream, _) = listener.accept().await?;
                let uid = owned.uid();
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = timeout(Duration::from_secs(3), handle(stream, uid)).await;
                });
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        };
        tokio::select! {
            result = serving => result,
            _ = terminate.recv() => Ok(()),
            _ = interrupt.recv() => Ok(()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_owner_and_privileged_local_product_can_use_private_socket() {
        assert!(allowed_peer(1000,1000));
        assert!(allowed_peer(1000,0));
        assert!(!allowed_peer(1000,1001));
    }
    #[test]
    fn stateless_call_keeps_ids_and_does_not_learn_other_call_topology() {
        let result = dispatch("/v1/normalize", br#"{"local_host_id":"a","known_hosts":["b"],"hosts":"b","request_id":"keep","origin_host":"a","hop_count":0}"#).unwrap();
        assert_eq!(result["hosts"], json!(["b"])); assert_eq!(result["request_id"], "keep");
        assert!(dispatch("/v1/normalize", br#"{"local_host_id":"c","known_hosts":[],"hosts":"b"}"#).is_err());
    }
    #[test]
    fn forwarded_wildcard_and_overflow_are_rejected() {
        assert!(dispatch("/v1/normalize", br#"{"local_host_id":"a","known_hosts":["b"],"hosts":"*","hop_count":1}"#).is_err());
        let peers:Vec<_>=(0..101).map(|n| n.to_string()).collect();
        assert!(dispatch("/v1/normalize", &serde_json::to_vec(&json!({"local_host_id":"a","known_hosts":peers})).unwrap()).is_err());
    }
}
