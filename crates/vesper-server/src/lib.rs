#![forbid(unsafe_code)]
//! Vesper server: HTTP + WebSocket signaling and presence in front of the audio SFU.

pub mod api_accounts;
pub mod api_admin;
pub mod api_rooms;
pub mod auth;
pub mod cli;
pub mod config;
pub mod limits;
pub mod public_addr;
pub mod rooms;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

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
use vesper_sfu::{Sfu, SfuConfig, SfuHandle};

use axum::extract::DefaultBodyLimit;
use axum::http::Method;
use vesper_protocol::ApiErrorCode;
use vesper_store::Store;

pub use crate::api_accounts::{ApiError, Authed};
use crate::config::Config;
use crate::limits::Limiter;
use crate::rooms::Rooms;

/// Largest JSON body any `/api` route reads.
pub const API_BODY_LIMIT: usize = 32 * 1024;

/// Shared state of the `/api` routes.
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub cfg: Arc<Config>,
    pub limits: Limiter,
}

/// The `/api` routes, with the Origin check and the body cap.
pub fn router(state: AppState) -> Router {
    let api = api_accounts::routes()
        .merge(api_admin::routes())
        .merge(api_rooms::routes())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_origin,
        ))
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT));
    Router::new().nest("/api", api).with_state(state)
}

/// CSRF guard: every `/api` request that can change state must come from our own origin.
async fn require_origin(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD);
    if !safe && !auth::origin_ok(req.headers(), &st.cfg.origin) {
        return ApiError::new(ApiErrorCode::Forbidden, "request from another origin")
            .into_response();
    }
    next.run(req).await
}

/// Start the SFU and the HTTP server. Returns the bound HTTP address and the serving task.
pub async fn start(cfg: Config) -> std::io::Result<(SocketAddr, JoinHandle<std::io::Result<()>>)> {
    let sfu = Sfu::spawn(SfuConfig {
        bind: cfg.udp_bind,
        ..Default::default()
    })?;
    // Not in start_with_sfu: tests crash the SFU on purpose and must survive it.
    watch_sfu(sfu.clone(), SFU_POLL, || {
        tracing::error!("voice engine (SFU thread) died; exiting so the supervisor restarts us");
        std::process::exit(1);
    });
    start_with_sfu(cfg, sfu).await
}

const SFU_POLL: Duration = Duration::from_secs(1);

/// Poll `sfu.is_alive()` every `interval`; run `on_dead` once when it turns false.
/// is_alive only goes false when the SFU thread exits, and this watcher's own handle clone keeps
/// the thread from ending by "all handles dropped", so a normal shutdown (process exit or
/// runtime teardown, which just drops this task) never fires it.
pub fn watch_sfu(sfu: SfuHandle, interval: Duration, on_dead: impl FnOnce() + Send + 'static) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        while sfu.is_alive() {
            tick.tick().await;
        }
        on_dead();
    });
}

/// [`start`] with an SFU the caller already spawned (`cfg.udp_bind` is then unused).
pub async fn start_with_sfu(
    cfg: Config,
    sfu: SfuHandle,
) -> std::io::Result<(SocketAddr, JoinHandle<std::io::Result<()>>)> {
    match cfg.public_host.clone() {
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
    let db = cfg.data_dir.join("vesper.db");
    let store = Store::open(&db)
        .await
        .map_err(|e| std::io::Error::other(format!("open {}: {e}", db.display())))?;
    let rooms = Rooms::new(sfu, &cfg.dev_key, cfg.max_room_size);
    let api = router(AppState {
        store,
        cfg: Arc::new(cfg.clone()),
        limits: Limiter::default(),
    });
    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws::upgrade))
        // ServeDir refuses `..` and never lists directories; SPA fallback is done in `guard`.
        .fallback_service(ServeDir::new(&cfg.web_dir))
        .with_state(ws::WsState {
            rooms,
            sockets: Arc::new(Semaphore::new(cfg.max_sockets)),
            join_deadline: cfg.join_deadline,
            ping_interval: cfg.ping_interval,
        })
        .merge(api)
        .layer(middleware::from_fn_with_state(
            Arc::new(cfg.web_dir.clone()),
            guard,
        ));
    let listener = tokio::net::TcpListener::bind(cfg.http_bind).await?;
    let addr = listener.local_addr()?;
    info!("listening on {addr}");
    Ok((
        addr,
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
        }),
    ))
}

/// 503 once the SFU thread has died, for external monitors and load balancers. (Docker does not
/// restart an unhealthy container; `start` also exits the process so restart policies recover.)
async fn health(State(st): State<ws::WsState>) -> (StatusCode, &'static str) {
    if st.rooms.sfu_alive() {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "sfu down")
    }
}

/// Static-serving policy around ServeDir:
/// - dotfiles are hidden (a served dir may hold `.env`);
/// - a 404 on an extensionless path gets `index.html` (SPA routes); a 404 on a path with an
///   extension (`/assets/old.js`) or under `/api/` stays a 404 so it fails loudly, not as HTML;
/// - `/assets/*` (content-hashed by Vite) is immutable, `/api/*` is `no-store` (per-user data),
///   everything else is `no-cache`;
/// - `nosniff` on every response.
async fn guard(State(dir): State<Arc<PathBuf>>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_ascii_lowercase();
    if path.contains("/.") || path.contains("%2e") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut res = next.run(req).await;
    let spa = res.status() == StatusCode::NOT_FOUND
        && !path.starts_with("/api/")
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
        } else if path.starts_with("/api/") {
            "no-store"
        } else {
            "no-cache"
        }),
    );
    res
}
