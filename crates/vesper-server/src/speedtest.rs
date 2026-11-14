//! `POST /api/speedtest`: the share settings dialog posts 4 MiB and times it to suggest a
//! bitrate. The body is read under the upload limits and thrown away; nothing is stored.

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;

use crate::api_accounts::limit;
use crate::limits::Kind;
use crate::uploads::{READ_IDLE, READ_TOTAL, read_body};
use crate::{ApiError, AppState, Authed};

pub fn routes() -> Router<AppState> {
    Router::new().route("/speedtest", post(upload))
}

async fn upload(State(st): State<AppState>, a: Authed, body: Body) -> Result<StatusCode, ApiError> {
    limit(&st, Kind::Speedtest, &a.user.user_id)?;
    let max = st.cfg.upload_max_bytes;
    read_body(body, &mut tokio::io::sink(), max, READ_IDLE, READ_TOTAL).await?;
    Ok(StatusCode::NO_CONTENT)
}
