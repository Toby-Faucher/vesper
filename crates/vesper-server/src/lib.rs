#![forbid(unsafe_code)]
//! Vesper server: HTTP + WebSocket signaling and presence in front of the audio SFU.

pub mod api_accounts;
pub mod api_admin;
pub mod api_messages;
pub mod api_push;
pub mod api_rooms;
pub mod auth;
pub mod avatars;
pub mod cli;
pub mod config;
pub mod hub;
pub mod limits;
pub mod public_addr;
pub mod push;
pub mod uploads;
pub mod voice;
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
use crate::hub::Hub;
use crate::limits::Limiter;
use crate::push::Push;
use crate::voice::Voice;

/// The release number (the workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The build id compiled in (`build.rs`): the git commit, else the version.
pub const BUILD: &str = env!("VESPER_BUILD");

/// When [`serve`] started, for the uptime on Admin > Server.
pub(crate) static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Largest JSON body any `/api` route reads.
pub const API_BODY_LIMIT: usize = 32 * 1024;

/// Shared state of the `/api` routes.
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub cfg: Arc<Config>,
    pub limits: Limiter,
    pub hub: Hub,
    pub voice: Voice,
    pub push: Push,
}

/// The `/api` routes, with the Origin check and the body cap.
pub fn router(state: AppState) -> Router {
    let api = api_accounts::routes()
        .merge(api_admin::routes())
        .merge(api_rooms::routes())
        .merge(api_messages::routes())
        .merge(api_push::routes())
        .merge(uploads::routes())
        .merge(avatars::routes())
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
    let db = cfg.data_dir.join("vesper.db");
    backup_before_migration(&cfg, &db).await?;
    let store = Store::open(&db)
        .await
        .map_err(|e| std::io::Error::other(format!("open {}: {e}", db.display())))?;
    if cfg.backup_every_hours > 0 {
        spawn_backups(cfg.clone());
    }
    let hub = Hub::default();
    let voice = Voice::new(sfu.clone(), hub.clone(), cfg.max_room_size);
    let push = Push::new(store.clone(), hub.clone(), cfg.push_subject.clone())
        .await
        .map_err(std::io::Error::other)?;
    if push.public_key().is_none() {
        info!("web push is off: VESPER_ORIGIN is not https and VESPER_PUSH_SUBJECT is unset");
    }
    let state = AppState {
        store,
        push,
        cfg: Arc::new(cfg),
        limits: Limiter::default(),
        hub,
        voice,
    };
    serve(state, sfu).await
}

/// When the database exists and this binary carries newer migrations, copy it into
/// `<data>/backups` first ([`cli::auto_backup`]). A failed copy stops the start: the data is
/// worth more than the uptime.
pub async fn backup_before_migration(cfg: &Config, db: &std::path::Path) -> std::io::Result<()> {
    let versions = Store::schema_versions(db)
        .await
        .map_err(|e| std::io::Error::other(format!("read {}: {e}", db.display())))?;
    // 0: a new or empty file, nothing to lose.
    let Some((from, to)) = versions.filter(|&(from, to)| from > 0 && from < to) else {
        return Ok(());
    };
    let kind = cli::Auto::PreMigration { from, to };
    match cli::auto_backup(cfg, kind, vesper_store::now_ms()).await {
        Ok(p) => {
            info!(
                "database schema {from} to {to}: backed up to {} first",
                p.display()
            );
            Ok(())
        }
        Err(e) => Err(std::io::Error::other(format!(
            "the backup before migrating the database (schema {from} to {to}) failed, so the server does not start: {e}"
        ))),
    }
}

/// A backup every `VESPER_BACKUP_EVERY` hours, the first one that long after start. A failure is
/// logged; the server keeps running.
fn spawn_backups(cfg: Config) {
    let every = Duration::from_secs(cfg.backup_every_hours.saturating_mul(3600));
    tokio::spawn(async move {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        loop {
            tick.tick().await;
            match cli::auto_backup(&cfg, cli::Auto::Scheduled, vesper_store::now_ms()).await {
                Ok(p) => info!("scheduled backup written to {}", p.display()),
                Err(e) => tracing::error!("scheduled backup failed: {e}"),
            }
        }
    });
}

/// Serve the whole app for `state` on `state.cfg.http_bind`: `/api`, `/ws`, `/health` and the
/// web app.
pub async fn serve(
    state: AppState,
    sfu: SfuHandle,
) -> std::io::Result<(SocketAddr, JoinHandle<std::io::Result<()>>)> {
    let cfg = state.cfg.clone();
    STARTED.get_or_init(std::time::Instant::now);
    uploads::clean_tmp(&cfg.data_dir)?;
    uploads::spawn_sweep(state.clone());
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
    let api = router(state.clone());
    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws::upgrade))
        // ServeDir refuses `..` and never lists directories; SPA fallback is done in `guard`. It sends
        // the `.br` or `.gz` beside a file (web/scripts/precompress.mjs) when Accept-Encoding allows,
        // with the original's Content-Type and `Vary: accept-encoding`.
        .fallback_service(
            ServeDir::new(&cfg.web_dir)
                .precompressed_br()
                .precompressed_gzip(),
        )
        .with_state(ws::WsState {
            app: state,
            sockets: Arc::new(Semaphore::new(cfg.max_sockets)),
            ping_interval: cfg.ping_interval,
        })
        .merge(api)
        .layer(middleware::from_fn_with_state(
            Arc::new(Guard {
                web_dir: cfg.web_dir.clone(),
                csp: HeaderValue::from_str(&csp(&cfg.origin)).map_err(std::io::Error::other)?,
                https: cfg.origin.starts_with("https://"),
            }),
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
/// The body is `{"status":"ok","version":..,"build":..}` (`"sfu down"` with the 503).
async fn health(State(st): State<ws::WsState>) -> (StatusCode, axum::Json<serde_json::Value>) {
    let (code, status) = if st.app.voice.sfu_alive() {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "sfu down")
    };
    let b = st.app.cfg.server_build();
    let body = serde_json::json!({ "status": status, "version": b.version, "build": b.build });
    (code, axum::Json(body))
}

/// The page's Content-Security-Policy (docs/decisions/0008-csp.md), for the
/// normalised `origin`. `connect-src` names the WebSocket origin as well as `'self'`, because
/// some older browsers do not let `'self'` match `ws:` and `wss:`. `'wasm-unsafe-eval'` lets the
/// DSP worker compile the denoiser; `'unsafe-inline'` is for styles only (Svelte writes `style`
/// attributes). Scripts stay strict: no `'unsafe-eval'`, no inline script.
///
/// Panics unless `origin` is already a normalised origin: it is pasted into the policy, so a
/// `;` or quote in it would add a directive or a source.
pub fn csp(origin: &str) -> String {
    assert!(
        config::normalise_origin(origin).as_deref() == Ok(origin),
        "csp: not a normalised origin"
    );
    let ws = format!("ws{}", origin.strip_prefix("http").unwrap_or(origin));
    format!(
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; \
         connect-src 'self' {ws}; img-src 'self' blob:; media-src 'self' blob:; \
         style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none'; \
         frame-ancestors 'none'; form-action 'self'"
    )
}

/// What `guard` needs: the web dir, the CSP header and whether the origin is https.
struct Guard {
    web_dir: PathBuf,
    csp: HeaderValue,
    https: bool,
}

/// Static-serving policy around ServeDir:
/// - dotfiles are hidden (a served dir may hold `.env`);
/// - a 404 on an extensionless path gets `index.html` (SPA routes); a 404 on a path with an
///   extension (`/assets/old.js`) or under `/api/` stays a 404 so it fails loudly, not as HTML;
/// - `/assets/*` (content-hashed by Vite) is immutable, `/api/*` is `no-store` (per-user data),
///   everything else is `no-cache`, unless the handler set its own (uploads);
/// - security headers on every response: `nosniff`, the CSP (a handler's own is kept, so an
///   upload keeps its sandbox), `Referrer-Policy`, `Permissions-Policy`, COOP, and HSTS when the
///   origin is https.
async fn guard(State(g): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_ascii_lowercase();
    if path.contains("/.") || path.contains("%2e") {
        return with_security_headers(StatusCode::NOT_FOUND.into_response(), &g);
    }
    let mut res = next.run(req).await;
    let spa = res.status() == StatusCode::NOT_FOUND
        && !path.starts_with("/api/")
        && !path.rsplit('/').next().unwrap_or("").contains('.');
    if spa && let Ok(html) = tokio::fs::read(g.web_dir.join("index.html")).await {
        res = ([(header::CONTENT_TYPE, "text/html")], html).into_response();
    }
    let immutable = path.starts_with("/assets/") && res.status() == StatusCode::OK;
    // Kept when the handler set it (`/api/uploads/*`).
    res.headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert_with(|| {
            HeaderValue::from_static(if immutable {
                "public, max-age=31536000, immutable"
            } else if path.starts_with("/api/") {
                "no-store"
            } else {
                "no-cache"
            })
        });
    with_security_headers(res, &g)
}

fn with_security_headers(mut res: Response, g: &Guard) -> Response {
    let h = res.headers_mut();
    let fixed = [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (
            header::HeaderName::from_static("permissions-policy"),
            "microphone=(self), camera=(), geolocation=()",
        ),
        (
            header::HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ),
    ];
    for (name, value) in fixed {
        h.insert(name, HeaderValue::from_static(value));
    }
    // An upload's `default-src 'none'; sandbox` is stricter than the page's: never replace it.
    h.entry(header::CONTENT_SECURITY_POLICY)
        .or_insert_with(|| g.csp.clone());
    if g.https {
        h.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    res
}
