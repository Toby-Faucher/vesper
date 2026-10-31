#![forbid(unsafe_code)]
//! Vesper server: HTTP + WebSocket signaling and presence in front of the audio SFU.

pub mod config;
pub mod public_addr;
pub mod rooms;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vesper_sfu::{Sfu, SfuConfig};

use crate::config::Config;
use crate::rooms::Rooms;

/// Start the SFU and the HTTP server. Returns the bound HTTP address and the serving task.
pub async fn start(cfg: Config) -> std::io::Result<(SocketAddr, JoinHandle<std::io::Result<()>>)> {
    let sfu = Sfu::spawn(SfuConfig {
        bind: cfg.udp_bind,
        ..Default::default()
    })?;
    match cfg.public_host {
        Some(host) => {
            public_addr::spawn_refresh(sfu.clone(), host, cfg.udp_port_public, public_addr::REFRESH)
        }
        None => {
            let local = sfu.local_addr();
            if local.ip().is_unspecified() {
                warn!(
                    "VESPER_PUBLIC_HOST unset and UDP bound to {local}: clients cannot reach media"
                );
            }
            sfu.set_advertised(vec![local]);
        }
    }
    let rooms = Rooms::new(sfu, &cfg.dev_key, cfg.max_room_size);
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(ws::upgrade))
        .with_state(ws::WsState {
            rooms,
            sockets: Arc::new(Semaphore::new(cfg.max_sockets)),
            join_deadline: cfg.join_deadline,
            ping_interval: cfg.ping_interval,
        });
    let listener = tokio::net::TcpListener::bind(cfg.http_bind).await?;
    let addr = listener.local_addr()?;
    info!("listening on {addr}");
    Ok((
        addr,
        tokio::spawn(async move { axum::serve(listener, app).await }),
    ))
}
