//! Profile pictures: `PUT /api/me/avatar` takes the raw bytes (at most 512 KiB; the web app
//! sends a 256 px square), `DELETE` removes it, and `GET /api/users/{id}/avatar` serves it to
//! any signed-in member (everyone sees the user list). The bytes live in the database, not with
//! the uploads: one small picture per account, gone with its row. The type is sniffed from the
//! bytes (png, jpeg, webp only; never svg or gif). The header must give a size of at most
//! `MAX_SIDE` px a side, so a small file cannot unpack to gigabytes in every member's browser. A png
//! must end at IEND and a webp at its RIFF size; a jpeg is only checked to end with FFD9. EXIF and
//! png text chunks are refused (the web app's canvas writes neither). The real guard against a
//! picture acting as a page is how it is served: the sniffed type, nosniff and a sandbox CSP, like
//! uploads. The markup denylist is only a second layer and easy to get round; do not grow it.

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use vesper_protocol::{ApiErrorCode, UserInfo};

use crate::api_accounts::limit;
use crate::api_admin::path;
use crate::limits::Kind;
use crate::uploads::sniff_image;
use crate::{ApiError, AppState, Authed};

pub const MAX_AVATAR: usize = 512 * 1024;
/// Largest width or height. The web app sends 256.
pub const MAX_SIDE: u32 = 1024;

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
    Router::new()
        .route("/me/avatar", put(set).delete(clear))
        .route("/users/{id}/avatar", get(serve))
}

/// The picture's type when `b` is a png, jpeg or webp that passes the checks in the module doc;
/// otherwise None.
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
    let (w, h) = size(b, mime)?;
    let fits = (1..=MAX_SIDE).contains(&w) && (1..=MAX_SIDE).contains(&h);
    (whole && !markup && fits).then_some(mime)
}

/// Width and height from the header, or None when it cannot be read, or holds EXIF.
fn size(b: &[u8], mime: &str) -> Option<(u32, u32)> {
    let at = |i: usize, n: usize| b.get(i..i.checked_add(n)?);
    let be16 = |i| at(i, 2).map(|s| u32::from(u16::from_be_bytes([s[0], s[1]])));
    let le16 = |i| at(i, 2).map(|s| u32::from(u16::from_le_bytes([s[0], s[1]])));
    match mime {
        // IHDR must be the first chunk.
        "image/png" if at(12, 4)? == b"IHDR" => {
            let be32 = |i| at(i, 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]));
            Some((be32(16)?, be32(20)?))
        }
        "image/webp" => match at(12, 4)? {
            // Flags 0x08: EXIF.
            b"VP8X" if b.get(20)? & 0x08 == 0 => {
                let le24 = |i| at(i, 3).map(|s| u32::from_le_bytes([s[0], s[1], s[2], 0]) + 1);
                Some((le24(24)?, le24(27)?))
            }
            b"VP8 " if at(23, 3)? == b"\x9d\x01\x2a" => {
                Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff))
            }
            b"VP8L" if *b.get(20)? == 0x2f => {
                let s = at(21, 4)?;
                let v = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                Some(((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1))
            }
            _ => None,
        },
        // Walks the segments to the first frame header (SOFn). APP1 (EXIF), a scan or the end
        // before it is refused.
        "image/jpeg" => {
            let mut i = 2;
            loop {
                if *b.get(i)? != 0xff {
                    return None;
                }
                match *b.get(i + 1)? {
                    0xff => i += 1, // fill byte
                    0x01 | 0xd0..=0xd7 => i += 2,
                    0xe1 | 0xd9 | 0xda => return None,
                    0xc4 | 0xc8 | 0xcc => i += 2 + be16(i + 2)? as usize,
                    0xc0..=0xcf => return Some((be16(i + 7)?, be16(i + 5)?)),
                    _ => i += 2 + be16(i + 2)? as usize,
                }
            }
        }
        _ => None,
    }
}

/// Walks the chunks after the signature: the file must end right after IEND, and text and EXIF
/// chunks are refused.
fn png_ends_at_iend(b: &[u8]) -> bool {
    let mut i = 8;
    loop {
        let Some(head) = b.get(i..i + 8) else {
            return false;
        };
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = &head[4..8];
        if matches!(kind, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf") {
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
    vesper_store::hex(&Sha256::digest(b)[..8])
}

async fn set(
    State(st): State<AppState>,
    a: Authed,
    body: Body,
) -> Result<Json<UserInfo>, ApiError> {
    // Before the body is read, so a limited client cannot make the server buffer 512 KiB.
    limit(&st, Kind::Upload, &a.user.user_id)?;
    let body = Limited::new(body, MAX_AVATAR)
        .collect()
        .await
        .map_err(|e| {
            if e.is::<LengthLimitError>() {
                ApiError::new(ApiErrorCode::TooLarge, "picture over 512 KiB")
            } else {
                ApiError::new(ApiErrorCode::BadRequest, "upload interrupted")
            }
        })?
        .to_bytes();
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

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut ihdr = [0u8; 13];
        ihdr[..4].copy_from_slice(&w.to_be_bytes());
        ihdr[4..8].copy_from_slice(&h.to_be_bytes());
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [(&b"IHDR"[..], &ihdr[..]), (b"IDAT", b"xx"), (b"IEND", b"")] {
            b.extend((data.len() as u32).to_be_bytes());
            b.extend(kind);
            b.extend(data);
            b.extend([0; 4]);
        }
        b
    }

    /// SOI, an APP0, then `seg` (a frame header or anything else), a scan and EOI.
    fn jpeg(seg: &[u8]) -> Vec<u8> {
        let mut b = b"\xff\xd8\xff\xe0\0\x06JFIF".to_vec();
        b.extend(seg);
        b.extend(b"\xff\xda\0\x02data\xff\xd9");
        b
    }

    fn sof(w: u16, h: u16) -> Vec<u8> {
        let mut s = b"\xff\xc0\0\x11\x08".to_vec();
        s.extend(h.to_be_bytes());
        s.extend(w.to_be_bytes());
        s.extend([3; 10]);
        s
    }

    fn webp(chunk: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
        b.extend(chunk);
        b.extend((payload.len() as u32).to_le_bytes());
        b.extend(payload);
        let riff = (b.len() - 8) as u32;
        b[4..8].copy_from_slice(&riff.to_le_bytes());
        b
    }

    fn vp8x(w: u32, h: u32, flags: u8) -> Vec<u8> {
        let mut p = vec![flags, 0, 0, 0];
        p.extend(&(w - 1).to_le_bytes()[..3]);
        p.extend(&(h - 1).to_le_bytes()[..3]);
        webp(b"VP8X", &p)
    }

    fn vp8(w: u16, h: u16) -> Vec<u8> {
        let mut p = b"\0\0\0\x9d\x01\x2a".to_vec();
        p.extend(w.to_le_bytes());
        p.extend(h.to_le_bytes());
        webp(b"VP8 ", &p)
    }

    fn vp8l(w: u32, h: u32) -> Vec<u8> {
        let mut p = vec![0x2f];
        p.extend(((w - 1) | (h - 1) << 14).to_le_bytes());
        webp(b"VP8L", &p)
    }

    #[test]
    fn avatar_mime_rules() {
        assert_eq!(avatar_mime(&png(256, 256)), Some("image/png"));
        assert_eq!(avatar_mime(&jpeg(&sof(256, 256))), Some("image/jpeg"));
        assert_eq!(avatar_mime(&vp8x(256, 256, 0)), Some("image/webp"));

        let mut trailing = png(256, 256);
        trailing.extend(b"<p>hi");
        let mut text = png(256, 256);
        text.splice(33..33, *b"\0\0\0\x01tEXta\0\0\0\0");
        let mut exif = png(256, 256);
        exif.splice(33..33, *b"\0\0\0\x01eXIfa\0\0\0\0");
        let mut script = png(256, 256);
        script.splice(33..33, *b"\0\0\0\x08IDAT<ScRiPt\0\0\0\0");
        let mut cut = png(256, 256);
        cut.truncate(cut.len() - 1);
        let mut webp_tail = vp8x(256, 256, 0);
        webp_tail.extend(b"more");
        let mut webp_long = vp8x(256, 256, 0);
        webp_long[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let jpeg_markup = jpeg(&[&b"\xff\xfe\0\x08<html>"[..], &sof(256, 256)].concat());
        let mut jpeg_tail = jpeg(&sof(256, 256));
        jpeg_tail.extend(b"<html>");
        for (no, why) in [
            (&b"GIF89a\0\0\0\0"[..], "gif"),
            (b"<svg xmlns='http://www.w3.org/2000/svg'/>", "svg"),
            (b"hello", "text"),
            (b"", "empty"),
            (&trailing, "bytes after IEND"),
            (&text, "text chunk"),
            (&exif, "png EXIF chunk"),
            (&script, "markup in a chunk"),
            (&cut, "cut short"),
            (&jpeg_markup, "jpeg with markup"),
            (&jpeg_tail, "jpeg with a tail"),
            (&webp_tail, "webp with a tail"),
            (&webp_long, "webp size past the end"),
        ] {
            assert_eq!(avatar_mime(no), None, "{why}");
        }
    }

    /// A small file must not declare a picture that unpacks to gigabytes: at most 1024 a side.
    #[test]
    fn avatar_size_is_capped_in_every_format() {
        type Make = fn(u32) -> Vec<u8>;
        let cases: [(&str, Make); 4] = [
            ("png", |n| png(n, n)),
            ("jpeg", |n| jpeg(&sof(n as u16, n as u16))),
            ("webp VP8X", |n| vp8x(n, n, 0)),
            ("webp VP8L", |n| vp8l(n, n)),
        ];
        for (name, make) in cases {
            assert!(avatar_mime(&make(256)).is_some(), "{name} 256");
            assert!(avatar_mime(&make(1024)).is_some(), "{name} 1024");
            assert_eq!(avatar_mime(&make(1025)), None, "{name} 1025");
            assert_eq!(avatar_mime(&make(16000)), None, "{name} 16000");
        }
        // One side over is enough.
        assert_eq!(avatar_mime(&png(1025, 256)), None);
        assert_eq!(avatar_mime(&png(256, 1025)), None);
        assert_eq!(avatar_mime(&png(256, 20000)), None);
        assert_eq!(avatar_mime(&png(20000, 20000)), None);
        assert_eq!(avatar_mime(&png(0, 256)), None);
        assert_eq!(avatar_mime(&vp8x(20000, 20000, 0)), None);
        assert_eq!(avatar_mime(&jpeg(&sof(256, 20000))), None);
        // VP8 (lossy) keeps 14 bits; the top two are a scale.
        assert!(avatar_mime(&vp8(256, 256)).is_some());
        assert!(avatar_mime(&vp8(1024, 1024)).is_some());
        assert_eq!(avatar_mime(&vp8(1025, 1025)), None);
        assert_eq!(avatar_mime(&vp8(16383, 16383)), None);
        assert!(
            avatar_mime(&vp8(256 | 0xc000, 256 | 0x4000)).is_some(),
            "scale bits"
        );
    }

    #[test]
    fn avatar_header_must_be_readable() {
        // PNG: IHDR first, whole.
        let mut late = png(256, 256);
        late[12..16].copy_from_slice(b"IDAT");
        assert_eq!(avatar_mime(&late), None, "png without IHDR first");
        assert_eq!(
            avatar_mime(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0"),
            None,
            "png cut in IHDR"
        );
        // JPEG: a frame header before any scan, whole, and no EXIF.
        assert_eq!(avatar_mime(&jpeg(b"")), None, "jpeg with no SOF");
        assert_eq!(
            avatar_mime(b"\xff\xd8\xff\xc0\0\x11\x08\x01\xff\xd9"),
            None,
            "jpeg cut in SOF"
        );
        let exif = jpeg(&[&b"\xff\xe1\0\x06Exif"[..], &sof(256, 256)].concat());
        assert_eq!(avatar_mime(&exif), None, "jpeg EXIF");
        assert_eq!(
            avatar_mime(&jpeg(b"\xff\xfe\xff\xff")),
            None,
            "jpeg segment past the end"
        );
        let with_dht = jpeg(&[&b"\xff\xc4\0\x03\0"[..], &sof(256, 256)].concat());
        assert!(
            avatar_mime(&with_dht).is_some(),
            "DHT (C4) is not a frame header"
        );
        // WebP: a known first chunk, whole, and no EXIF flag.
        assert_eq!(avatar_mime(&vp8x(256, 256, 0x08)), None, "webp EXIF flag");
        assert_eq!(
            avatar_mime(&webp(b"ALPH", b"0123456789")),
            None,
            "webp unknown chunk"
        );
        assert_eq!(
            avatar_mime(&webp(b"VP8X", b"\0\0\0\0\xff")),
            None,
            "webp cut in VP8X"
        );
        assert_eq!(
            avatar_mime(&webp(b"VP8 ", b"\0\0\0\0\0\0\x01\x01\x01\x01")),
            None,
            "vp8 no start code"
        );
        assert_eq!(
            avatar_mime(&webp(b"VP8L", b"\x00\0\0\0\0")),
            None,
            "vp8l no signature"
        );
    }
}
