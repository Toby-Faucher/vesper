#![forbid(unsafe_code)]
//! Vesper server: HTTP + WebSocket signaling and presence in front of the audio SFU.

pub mod config;
pub mod public_addr;
pub mod rooms;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tower_http::services::{ServeDir, ServeFile};
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
        .fallback_service(
            // SPA: unknown paths get index.html. ServeDir refuses `..` and never lists directories.
            ServeDir::new(&cfg.web_dir).fallback(ServeFile::new(cfg.web_dir.join("index.html"))),
        )
        .layer(middleware::from_fn(guard))
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

/// Hide dotfiles (a served dir may hold `.env` and friends) and add `nosniff` to every response.
async fn guard(req: Request, next: Next) -> Response {
    let p = req.uri().path().to_ascii_lowercase();
    if p.contains("/.") || p.contains("%2e") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut res = next.run(req).await;
    res.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}
