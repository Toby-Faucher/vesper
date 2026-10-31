#![forbid(unsafe_code)]
//! Vesper server: HTTP + WebSocket signaling and presence in front of the audio SFU.

pub mod config;
pub mod public_addr;
pub mod rooms;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use std::path::PathBuf;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tower_http::services::ServeDir;
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
    if !cfg.web_dir.join("index.html").is_file() {
        warn!(
            "{}/index.html not found: the web app will 404 (set VESPER_WEB_DIR to the built web/dist)",
            cfg.web_dir.display()
        );
    }
    let rooms = Rooms::new(sfu, &cfg.dev_key, cfg.max_room_size);
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(ws::upgrade))
        // ServeDir refuses `..` and never lists directories; SPA fallback is done in `guard`.
        .fallback_service(ServeDir::new(&cfg.web_dir))
        .layer(middleware::from_fn_with_state(
            Arc::new(cfg.web_dir.clone()),
            guard,
        ))
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

/// Static-serving policy around ServeDir:
/// - dotfiles are hidden (a served dir may hold `.env`);
/// - a 404 on an extensionless path gets `index.html` (SPA routes); a 404 on a path with an
///   extension (`/assets/old.js`) stays a 404 so a stale page fails loudly, not as HTML;
/// - `/assets/*` (content-hashed by Vite) is immutable, everything else is `no-cache`;
/// - `nosniff` on every response.
async fn guard(State(dir): State<Arc<PathBuf>>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_ascii_lowercase();
    if path.contains("/.") || path.contains("%2e") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut res = next.run(req).await;
    let spa = res.status() == StatusCode::NOT_FOUND
        && !path.rsplit('/').next().unwrap_or("").contains('.');
    if spa && let Ok(html) = tokio::fs::read(dir.join("index.html")).await {
        res = ([(header::CONTENT_TYPE, "text/html")], html).into_response();
    }
    let immutable = path.starts_with("/assets/") && res.status() == StatusCode::OK;
    let h = res.headers_mut();
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    res
}
