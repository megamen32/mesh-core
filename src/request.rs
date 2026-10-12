use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum HostsInput {
    One(String),
    Many(Vec<String>),
}

impl HostsInput {
    fn into_vec(self) -> Vec<String> {
        match self {
            HostsInput::One(v) => vec![v],
            HostsInput::Many(v) => v,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NormalizedRequest {
    pub request_id: String,
    pub origin_host: String,
    pub hop_count: u8,
    pub hosts: Vec<String>,
}

pub fn normalize_request(
    local_host_id: &str,
    known_hosts: impl IntoIterator<Item = String>,
    hosts: Option<HostsInput>,
    request_id: Option<String>,
    origin_host: Option<String>,
    hop_count: Option<u8>,
) -> Result<NormalizedRequest> {
    let hop_count = hop_count.unwrap_or(0);
    if hop_count > 1 {
        return Err(anyhow!("hop_count above one is rejected"));
    }

    let mut known: BTreeSet<String> = known_hosts.into_iter().collect();
    known.insert(local_host_id.to_string());
    let requested = hosts.unwrap_or(HostsInput::One("*".into())).into_vec();
    let request_id = request_id.unwrap_or_else(|| format!("{}-{}", local_host_id, fresh_id()));
    let origin_host = origin_host.unwrap_or_else(|| local_host_id.to_string());

    let hosts = if hop_count == 1 {
        if requested != vec!["local".to_string()] {
            return Err(anyhow!("peer calls are local-only"));
        }
        vec![local_host_id.to_string()]
    } else if requested.len() == 1 && requested[0] == "local" {
        vec![local_host_id.to_string()]
    } else if requested.len() == 1 && requested[0] == "*" {
        known.into_iter().collect()
    } else {
        let mut dedup = BTreeSet::new();
        let mut out = Vec::new();
        for host in requested {
            let target = if host == "local" {
                local_host_id.to_string()
            } else {
                host
            };
            if !known.contains(&target) {
                return Err(anyhow!("unknown host {}", target));
            }
            if dedup.insert(target.clone()) {
                out.push(target);
            }
        }
        out
    };

    Ok(NormalizedRequest {
        request_id,
        origin_host,
        hop_count,
        hosts,
    })
}

fn fresh_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:x}{:x}", d.as_secs(), d.subsec_nanos())
}
