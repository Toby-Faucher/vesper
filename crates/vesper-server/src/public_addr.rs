use std::net::SocketAddr;
use std::time::Duration;

use tracing::warn;
use vesper_sfu::SfuHandle;

/// How often the public hostname is re-resolved, so a changed home IP is picked up live.
pub const REFRESH: Duration = Duration::from_secs(60);

/// Resolve `host` to the addresses clients can reach the SFU on. Empty on DNS failure.
pub async fn resolve_public(host: &str, port: u16) -> Vec<SocketAddr> {
    match tokio::net::lookup_host((host, port)).await {
        Ok(addrs) => addrs.collect(),
        Err(e) => {
            warn!("resolving public host {host}: {e}");
            vec![]
        }
    }
}

/// Re-resolve `host` every `every` and hand the result to the SFU. A failed or empty
/// resolution keeps the previous addresses rather than advertising nothing.
/// Limit: `set_advertised` only affects later sessions, so after an IP change clients must rejoin.
pub fn spawn_refresh(sfu: SfuHandle, host: String, port: u16, every: Duration) {
    tokio::spawn(async move {
        let mut last = vec![];
        let mut tick = tokio::time::interval(every);
        loop {
            tick.tick().await;
            let addrs = resolve_public(&host, port).await;
            if !addrs.is_empty() && addrs != last {
                sfu.set_advertised(addrs.clone());
                last = addrs;
            }
        }
    });
}
