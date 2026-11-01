//! Profile pictures: `PUT /api/me/avatar` takes the raw bytes (at most 512 KiB; the web app
//! sends a 256 px square), `DELETE` removes it, and `GET /api/users/{id}/avatar` serves it to
//! any signed-in member (everyone sees the user list). The bytes live in the database, not with
//! the uploads: one small picture per account, gone with its row. The type is sniffed from the
//! bytes (png, jpeg, webp only; never svg or gif), the file must end where its format says, and a
//! file holding markup is refused, so a picture cannot double as a page. It is served with the
//! uploads' headers: nosniff and a sandbox CSP.

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, PathRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use vesper_protocol::{ApiErrorCode, UserInfo};

use crate::api_accounts::limit;
use crate::api_admin::path;
use crate::limits::Kind;
use crate::uploads::sniff_image;
use crate::{ApiError, AppState, Authed};

pub const MAX_AVATAR: usize = 512 * 1024;

/// Markup that would let a picture also parse as HTML, SVG or XML. Matched case-insensitively
/// anywhere in the file. Random compressed bytes hit one by chance about once in 10^5 pictures.
const MARKUP: [&[u8]; 9] = [
    b"<script",
    b"<html",
    b"<svg",
    b"<?xml",
    b"<!doctype",
    b"<iframe",
    b"<body",
    b"<object",
    b"<embed",
];

pub fn routes() -> Router<AppState> {
    let cap = DefaultBodyLimit::max(MAX_AVATAR);
    Router::new()
        .route("/me/avatar", put(set).delete(clear).layer(cap))
        .route("/users/{id}/avatar", get(serve))
}

/// The picture's type when `b` is a whole png, jpeg or webp with nothing after its end and no
/// markup inside; otherwise None.
pub fn avatar_mime(b: &[u8]) -> Option<&'static str> {
    let mime = sniff_image(b)?;
    let whole = match mime {
        "image/png" => png_ends_at_iend(b),
        "image/jpeg" => b.ends_with(&[0xff, 0xd9]),
        "image/webp" => {
            let riff = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
            riff.checked_add(8) == Some(b.len())
        }
        _ => false, // gif: not a profile picture
    };
    let markup = MARKUP
        .iter()
        .any(|m| b.windows(m.len()).any(|w| w.eq_ignore_ascii_case(m)));
    (whole && !markup).then_some(mime)
}

/// Walks the chunks after the signature: the file must end right after IEND, and text chunks
/// (a place to hide a script) are refused.
fn png_ends_at_iend(b: &[u8]) -> bool {
    let mut i = 8;
    loop {
        let Some(head) = b.get(i..i + 8) else {
            return false;
        };
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = &head[4..8];
        if matches!(kind, b"tEXt" | b"zTXt" | b"iTXt") {
            return false;
        }
        i = match i.checked_add(12).and_then(|n| n.checked_add(len)) {
            Some(n) if n <= b.len() => n,
            _ => return false,
        };
        if kind == b"IEND" {
            return i == b.len();
        }
    }
}

/// 16 hex of the bytes' SHA-256: the version in `UserInfo.avatar` and the ETag.
fn version(b: &[u8]) -> String {
    Sha256::digest(b)[..8]
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

async fn set(
    State(st): State<AppState>,
    a: Authed,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<UserInfo>, ApiError> {
    limit(&st, Kind::Upload, &a.user.user_id)?;
    let body = body.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(ApiErrorCode::TooLarge, "picture over 512 KiB")
        } else {
            ApiError::new(ApiErrorCode::BadRequest, "upload interrupted")
        }
    })?;
    let mime = avatar_mime(&body).ok_or_else(|| {
        ApiError::new(
            ApiErrorCode::BadRequest,
            "picture: a png, jpeg or webp image",
        )
    })?;
    let user = st
        .store
        .set_avatar(&a.user.user_id, mime, &body, &version(&body))
        .await?;
    st.hub.update_user(user.clone());
    Ok(Json(user))
}

async fn clear(State(st): State<AppState>, a: Authed) -> Result<Json<UserInfo>, ApiError> {
    limit(&st, Kind::Change, &a.user.user_id)?;
    let user = st.store.clear_avatar(&a.user.user_id).await?;
    st.hub.update_user(user.clone());
    Ok(Json(user))
}

#[derive(Deserialize)]
struct VersionQuery {
    v: Option<String>,
}

/// With `?v=` equal to the current version the answer is immutable (a new picture gets a new
/// URL); without it the browser revalidates with the ETag.
async fn serve(
    State(st): State<AppState>,
    _a: Authed,
    id: Result<Path<String>, PathRejection>,
    q: Result<Query<VersionQuery>, QueryRejection>,
    req: HeaderMap,
) -> Result<Response, ApiError> {
    let (mime, data, ver) = st.store.avatar(&path(id)?).await?;
    let etag = format!("\"{ver}\"");
    let fresh = req
        .get(header::IF_NONE_MATCH)
        .is_some_and(|v| v.as_bytes() == etag.as_bytes());
    let immutable = q.ok().and_then(|Query(q)| q.v).as_deref() == Some(ver.as_str());
    let mut res = if fresh {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        data.into_response()
    };
    let h = res.headers_mut();
    let fixed = [
        (header::CONTENT_TYPE, mime),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; sandbox".into(),
        ),
        (
            header::HeaderName::from_static("cross-origin-resource-policy"),
            "same-origin".into(),
        ),
        (header::ETAG, etag),
        (
            header::CACHE_CONTROL,
            if immutable {
                "private, max-age=31536000, immutable"
            } else {
                "private, no-cache"
            }
            .into(),
        ),
    ];
    for (k, v) in fixed {
        h.insert(k, HeaderValue::from_str(&v).expect("ascii header"));
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [
            (&b"IHDR"[..], &[0u8; 13][..]),
            (b"IDAT", b"xx"),
            (b"IEND", b""),
        ] {
            b.extend((data.len() as u32).to_be_bytes());
            b.extend(kind);
            b.extend(data);
            b.extend([0; 4]);
        }
        b
    }

    #[test]
    fn avatar_mime_rules() {
        assert_eq!(avatar_mime(&png()), Some("image/png"));
        let jpeg = b"\xff\xd8\xff\xe0JFIF\0rest\xff\xd9";
        assert_eq!(avatar_mime(jpeg), Some("image/jpeg"));
        let webp = b"RIFF\x08\0\0\0WEBPVP8 ";
        assert_eq!(avatar_mime(webp), Some("image/webp"));

        let mut trailing = png();
        trailing.extend(b"<p>hi");
        let mut text = png();
        text.splice(33..33, *b"\0\0\0\x01tEXta\0\0\0\0");
        let mut script = png();
        script.splice(33..33, *b"\0\0\0\x08IDAT<ScRiPt\0\0\0\0");
        let mut cut = png();
        cut.truncate(cut.len() - 1);
        for (no, why) in [
            (&b"GIF89a\0\0\0\0"[..], "gif"),
            (b"<svg xmlns='http://www.w3.org/2000/svg'/>", "svg"),
            (b"hello", "text"),
            (b"", "empty"),
            (&trailing, "bytes after IEND"),
            (&text, "text chunk"),
            (&script, "markup in a chunk"),
            (&cut, "cut short"),
            (b"\xff\xd8\xff\xe0<html>\xff\xd9", "jpeg with markup"),
            (b"\xff\xd8\xff\xe0JFIF\xff\xd9<html>", "jpeg with a tail"),
            (b"RIFF\x08\0\0\0WEBPVP8 more", "webp with a tail"),
            (b"RIFF\xff\xff\xff\xffWEBPVP8 ", "webp size past the end"),
        ] {
            assert_eq!(avatar_mime(no), None, "{why}");
        }
    }
}
