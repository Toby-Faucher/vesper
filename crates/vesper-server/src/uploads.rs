//! Uploads: a raw-body POST streamed to disk under a 25 MiB cap and a total quota, served back
//! with headers that keep a file from running as a page. Files live at
//! `<data dir>/uploads/<first 2 of id>/<id>` (dirs 0700, files 0600); the id is 32 random hex,
//! never anything the client sent. The type is sniffed from the first bytes: png, jpeg, gif and
//! webp are images; mp4, mov, m4a, webm, mp3, ogg, wav and flac are media; all of them are
//! served inline. Everything else is `application/octet-stream` and a download.
//! The client's file name is display text only.

use std::fmt::Write as _;
use std::path::{Path as FsPath, PathBuf};
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use http_body_util::BodyExt;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeFile;
use vesper_protocol::{ApiErrorCode, Attachment, AttachmentKind, DiskUse, Role, UploadDone};
use vesper_store::{StoreError, UploadRow, now_ms, random_hex};

use crate::api_accounts::limit;
use crate::api_admin::{bad, path, require};
use crate::api_rooms::visible_room;
use crate::auth::is_hidden_format;
use crate::limits::Kind;
use crate::{ApiError, AppState, Authed};

pub const MAX_UPLOAD: u64 = 25 * 1024 * 1024;
/// An upload not attached to a message within this long is swept.
pub const STALE_MS: i64 = 24 * 3600 * 1000;
const SWEEP_EVERY: Duration = Duration::from_secs(3600);
/// An upload body that sends nothing for this long, or is not done after `READ_TOTAL`, is dropped.
pub const READ_IDLE: Duration = Duration::from_secs(30);
pub const READ_TOTAL: Duration = Duration::from_secs(600);
const OCTET: &str = "application/octet-stream";
const IMAGES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const MEDIA: [&str; 8] = [
    "audio/mp4",
    "video/quicktime",
    "video/mp4",
    "video/webm",
    "audio/mpeg",
    "audio/ogg",
    "audio/wav",
    "audio/flac",
];
const MAX_NAME: usize = 200;

/// Held around the quota check, the rename and the row insert, so two uploads cannot both pass
/// the check with room for only one.
static QUOTA: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `uploads/<first 2>/<id>` under `data_dir`, or None unless `id` is 32 lowercase hex.
pub fn upload_path(data_dir: &FsPath, id: &str) -> Option<PathBuf> {
    let ok = id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    ok.then(|| data_dir.join("uploads").join(&id[..2]).join(id))
}

/// The image type whose magic bytes start `head`, if any.
pub fn sniff_image(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(IMAGES[0])
    } else if head.starts_with(b"\xff\xd8\xff") {
        Some(IMAGES[1])
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some(IMAGES[2])
    } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some(IMAGES[3])
    } else {
        None
    }
}

/// The audio or video type whose magic bytes start `head`, if any. The browser may still not
/// play it (a codec it lacks); the client then shows a download.
pub fn sniff_media(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize, m: &[u8]| head.get(i..i + m.len()) == Some(m);
    if at(4, b"ftyp") {
        // HEIF, AVIF and Canon raw share the ISO box; they stay a download.
        match head.get(8..12)? {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis"
            | b"crx " => None,
            b"M4A " | b"M4B " => Some(MEDIA[0]),
            b"qt  " => Some(MEDIA[1]),
            _ => Some(MEDIA[2]),
        }
    } else if at(0, b"\x1a\x45\xdf\xa3") {
        Some(MEDIA[3])
    } else if at(0, b"ID3") || mpeg_frame(head) {
        Some(MEDIA[4])
    } else if at(0, b"OggS") {
        Some(MEDIA[5])
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        Some(MEDIA[6])
    } else if at(0, b"fLaC") {
        Some(MEDIA[7])
    } else {
        None
    }
}

/// An MPEG audio frame header: 11 sync bits, then no reserved version, layer, bitrate or rate.
/// Layer bits 00 are AAC ADTS, not mp3. JPEG's `FF D8` fails the sync.
fn mpeg_frame(head: &[u8]) -> bool {
    let [0xff, b1, b2, ..] = *head else {
        return false;
    };
    b1 & 0xe0 == 0xe0 && b1 & 0x18 != 0x08 && b1 & 0x06 != 0 && b2 >> 4 != 0xf && b2 & 0x0c != 0x0c
}

/// The type an upload is stored and served as: an image or media type, or None for a download.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    sniff_image(head).or_else(|| sniff_media(head))
}

/// Display name: control, bidi and other hidden format characters dropped, `/` and `\` become
/// `_`, at most 200 chars, trimmed; empty becomes `file`.
pub fn clean_name(s: &str) -> String {
    let s: String = s
        .chars()
        .filter(|&c| {
            !c.is_control() && !is_hidden_format(c) && !matches!(c, '\u{2028}' | '\u{2029}')
        })
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .take(MAX_NAME)
        .collect();
    match s.trim() {
        "" => "file".into(),
        t => t.into(),
    }
}

/// `inline` or `attachment`, an ASCII `filename="..."` (anything but printable ASCII, `"` and
/// `\` become `_`) and the exact name as `filename*=UTF-8''<percent-encoded>` (RFC 6266, 5987).
pub fn content_disposition(name: &str, inline: bool) -> HeaderValue {
    let ascii: String = name
        .chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' => c,
            _ => '_',
        })
        .collect();
    let mut pct = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            pct.push(b as char);
        } else {
            let _ = write!(pct, "%{b:02X}");
        }
    }
    let kind = if inline { "inline" } else { "attachment" };
    HeaderValue::from_str(&format!(
        "{kind}; filename=\"{ascii}\"; filename*=UTF-8''{pct}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/rooms/{id}/uploads", post(upload))
        .route("/uploads/{id}", get(download))
        .route("/admin/disk", get(disk))
}

fn not_found() -> ApiError {
    ApiError::new(ApiErrorCode::NotFound, "not found")
}

fn too_large() -> ApiError {
    ApiError::new(ApiErrorCode::TooLarge, "file over 25 MiB")
}

fn quota_full() -> ApiError {
    ApiError::new(ApiErrorCode::QuotaFull, "upload quota is full")
}

fn io_error(e: std::io::Error) -> ApiError {
    tracing::error!("uploads: {e}");
    ApiError::new(ApiErrorCode::Internal, "server error")
}

/// Creates `dir` and missing parents with mode 0700 (existing ones keep theirs).
fn private_dir(dir: &FsPath) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Removes its file when dropped, unless `keep` ran. A request dropped mid-body (client gone)
/// is cleaned up too.
struct Cleanup(Option<PathBuf>);

impl Cleanup {
    fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Empties `uploads/tmp` (left by a crash) and creates it. Run at start.
pub fn clean_tmp(data_dir: &FsPath) -> std::io::Result<()> {
    let tmp = data_dir.join("uploads").join("tmp");
    match std::fs::remove_dir_all(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    private_dir(&tmp)
}

/// Deletes the files of `ids` (their rows are already gone). Errors are logged, not returned.
pub async fn remove_files(data_dir: &FsPath, ids: &[String]) {
    for p in ids.iter().filter_map(|id| upload_path(data_dir, id)) {
        if let Err(e) = tokio::fs::remove_file(&p).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("remove upload {}: {e}", p.display());
        }
    }
}

/// Removes uploads that are still unattached `STALE_MS` before `now`. Returns how many.
pub async fn sweep(st: &AppState, now: i64) -> Result<usize, StoreError> {
    let stale = st.store.stale_uploads(now - STALE_MS).await?;
    if stale.is_empty() {
        return Ok(0);
    }
    let gone = st.store.delete_uploads(&stale).await?;
    remove_files(&st.cfg.data_dir, &gone).await;
    Ok(gone.len())
}

/// Removes files under `uploads/<xx>/` last written `STALE_MS` before `now` that have no row: a
/// delete that failed, or a crash between the rename and the row insert. Returns how many. A file
/// or dir that fails (one deleted by a message delete mid-pass) is logged and skipped.
pub async fn sweep_orphans(st: &AppState, now: SystemTime) -> std::io::Result<usize> {
    let data_dir = &st.cfg.data_dir;
    let stale = Duration::from_millis(STALE_MS as u64);
    let mut dirs = match tokio::fs::read_dir(data_dir.join("uploads")).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        r => r?,
    };
    let mut old = Vec::new();
    while let Some(dir) = dirs.next_entry().await? {
        if dir.file_name() == "tmp" || !dir.file_type().await.is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if let Err(e) = old_files(data_dir, &dir.path(), now, stale, &mut old).await {
            tracing::warn!("orphan upload sweep: {}: {e}", dir.path().display());
        }
    }
    if old.is_empty() {
        return Ok(0);
    }
    // `upload` renames and inserts the row under this lock, so no row now means none is coming
    // for these ids (they are random, and a fresh rename has a new mtime anyway).
    let known = {
        let _quota = QUOTA.lock().await;
        st.store.known_uploads(&old).await
    }
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut n = 0;
    for id in old.iter().filter(|id| !known.contains(id)) {
        let Some(p) = upload_path(data_dir, id) else {
            continue;
        };
        match tokio::fs::remove_file(&p).await {
            Ok(()) => n += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("orphan upload sweep: {}: {e}", p.display()),
        }
    }
    Ok(n)
}

/// Pushes to `out` the ids of the upload files in `dir` last written `stale` before `now`.
/// A file that vanishes while it is read is skipped.
async fn old_files(
    data_dir: &FsPath,
    dir: &FsPath,
    now: SystemTime,
    stale: Duration,
    out: &mut Vec<String>,
) -> std::io::Result<()> {
    let mut files = tokio::fs::read_dir(dir).await?;
    while let Some(f) = files.next_entry().await? {
        let id = f.file_name().to_string_lossy().into_owned();
        // Only names this module writes, in their own dir.
        if upload_path(data_dir, &id).as_deref() != Some(f.path().as_path()) {
            continue;
        }
        match f.metadata().await.and_then(|m| m.modified()) {
            Ok(t) if t + stale <= now => out.push(id),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("orphan upload sweep: {}: {e}", f.path().display()),
        }
    }
    Ok(())
}

/// Runs `sweep` and `sweep_orphans` now and then every hour.
pub fn spawn_sweep(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            match sweep(&st, now_ms()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("swept {n} unattached uploads"),
                Err(e) => tracing::error!("upload sweep: {e}"),
            }
            match sweep_orphans(&st, SystemTime::now()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("removed {n} upload files with no row"),
                Err(e) => tracing::error!("orphan upload sweep: {e}"),
            }
        }
    });
}

/// Gives uploads stored as `application/octet-stream` the image or media type `sniff` now finds in
/// their first bytes, so files kept before the media sniffers (v1.8.0) play inline. A missing or
/// unreadable file is skipped; a row with any other type is never changed. Returns how many changed.
pub async fn retype_octet(st: &AppState) -> Result<usize, StoreError> {
    use tokio::io::AsyncReadExt;
    let mut n = 0;
    for id in st.store.octet_uploads().await? {
        let Some(p) = upload_path(&st.cfg.data_dir, &id) else {
            continue;
        };
        let mut head = Vec::with_capacity(16);
        let read = match tokio::fs::File::open(&p).await {
            Ok(f) => f.take(16).read_to_end(&mut head).await.map(drop),
            Err(e) => Err(e),
        };
        match read {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!("upload retype: {}: {e}", p.display());
                continue;
            }
        }
        if let Some(mime) = sniff(&head)
            && st.store.retype_upload(&id, mime).await?
        {
            n += 1;
        }
    }
    Ok(n)
}

/// Runs `retype_octet` once, in the background so start is not held up.
pub fn spawn_retype(st: AppState) {
    tokio::spawn(async move {
        match retype_octet(&st).await {
            Ok(n) => tracing::info!("gave {n} old uploads an image or media type"),
            Err(e) => tracing::error!("upload retype: {e}"),
        }
    });
}

#[derive(Deserialize)]
struct NameQuery {
    name: Option<String>,
}

async fn upload(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    q: Result<Query<NameQuery>, QueryRejection>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<UploadDone>), ApiError> {
    let room = visible_room(&st, &a.user, path(id)?).await?;
    let Query(q) = q.map_err(|_| bad("invalid query"))?;
    limit(&st, Kind::Upload, &a.user.user_id)?;
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_UPLOAD) {
        return Err(too_large());
    }
    // Saves reading a body that cannot fit. Only a hint: the check under QUOTA below decides.
    if let Some(n) = declared {
        let used = st.store.upload_bytes().await?.max(0) as u64;
        if used.saturating_add(n) > st.cfg.upload_quota_bytes {
            return Err(quota_full());
        }
    }

    let id = random_hex(16);
    let tmp_dir = st.cfg.data_dir.join("uploads").join("tmp");
    private_dir(&tmp_dir).map_err(io_error)?;
    let tmp = tmp_dir.join(&id);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .await
        .map_err(io_error)?;
    let mut cleanup = Cleanup(Some(tmp.clone()));
    let (size, head) = read_body(body, &mut file, READ_IDLE, READ_TOTAL).await?;
    file.sync_all().await.map_err(io_error)?;
    drop(file);

    let mime = sniff(&head);
    let row = UploadRow {
        id,
        room_id: room.id,
        uploader_id: a.user.user_id.clone(),
        message_id: None,
        name: clean_name(q.name.as_deref().unwrap_or("")),
        mime: mime.unwrap_or(OCTET).into(),
        size: size as i64,
        created_at: now_ms(),
    };
    let dest = upload_path(&st.cfg.data_dir, &row.id).expect("random_hex(16) is 32 lowercase hex");
    {
        let _quota = QUOTA.lock().await;
        let used = st.store.upload_bytes().await?.max(0) as u64;
        if used + size > st.cfg.upload_quota_bytes {
            return Err(quota_full());
        }
        private_dir(dest.parent().expect("has a parent")).map_err(io_error)?;
        tokio::fs::rename(&tmp, &dest).await.map_err(io_error)?;
        cleanup.0 = Some(dest);
        st.store.insert_upload(&row).await?;
        cleanup.keep();
    }
    let attachment = Attachment {
        kind: AttachmentKind::of(&row.mime),
        id: row.id,
        name: row.name,
        mime: row.mime,
        size: row.size,
    };
    Ok((StatusCode::CREATED, Json(UploadDone { attachment })))
}

/// Streams `body` into `file` under the 25 MiB cap. Returns the size and the first 16 bytes. A
/// gap of `idle` between chunks or a body not done after `total` is a 400, so a slow client
/// cannot hold an upload open.
pub async fn read_body(
    mut body: Body,
    file: &mut (impl tokio::io::AsyncWrite + Unpin),
    idle: Duration,
    total: Duration,
) -> Result<(u64, Vec<u8>), ApiError> {
    let deadline = tokio::time::Instant::now() + total;
    let mut size: u64 = 0;
    let mut head: Vec<u8> = Vec::with_capacity(16);
    loop {
        let next = (tokio::time::Instant::now() + idle).min(deadline);
        let frame = tokio::time::timeout_at(next, body.frame())
            .await
            .map_err(|_| bad("upload too slow"))?;
        let Some(frame) = frame else {
            return Ok((size, head));
        };
        let frame = frame.map_err(|_| bad("upload interrupted"))?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        size += chunk.len() as u64;
        if size > MAX_UPLOAD {
            return Err(too_large());
        }
        let want = (16 - head.len()).min(chunk.len());
        head.extend_from_slice(&chunk[..want]);
        file.write_all(&chunk).await.map_err(io_error)?;
    }
}

/// Attached: anyone who can see the message's room. Not attached yet: only the uploader.
/// Everything else is the same 404 as an unknown id.
async fn download(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<String>, PathRejection>,
    req: Request,
) -> Result<Response, ApiError> {
    let id = path(id)?;
    let file = upload_path(&st.cfg.data_dir, &id).ok_or_else(not_found)?;
    let row = st.store.upload(&id).await?;
    visible_room(&st, &a.user, row.room_id).await?;
    if row.message_id.is_none() && row.uploader_id != a.user.user_id {
        return Err(not_found());
    }
    let res = ServeFile::new(&file)
        .ignore_multi_range_requests(true)
        .try_call(req)
        .await
        .map_err(io_error)?;
    let mut res = res.map(Body::new);
    let known = IMAGES.into_iter().chain(MEDIA).find(|&m| m == row.mime);
    let ok = res.status().is_success();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(known.unwrap_or(OCTET)),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        content_disposition(&row.name, known.is_some()),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    h.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if ok {
            "private, max-age=31536000, immutable"
        } else {
            "no-store"
        }),
    );
    Ok(res)
}

async fn disk(State(st): State<AppState>, a: Authed) -> Result<Json<DiskUse>, ApiError> {
    require(&a.user, Role::Owner)?;
    disk_use(&st).await.map(Json)
}

/// The numbers of Admin > Disk use (also on Admin > Server).
pub(crate) async fn disk_use(st: &AppState) -> Result<DiskUse, ApiError> {
    let dir = &st.cfg.data_dir;
    let len = |name: &str| std::fs::metadata(dir.join(name)).map_or(0, |m| m.len());
    let db_bytes = len("vesper.db") + len("vesper.db-wal");
    let upload_bytes = st.store.upload_bytes().await?.max(0) as u64;
    let quota = st.cfg.upload_quota_bytes;
    let vfs = rustix::fs::statvfs(dir.as_path()).map_err(|e| io_error(e.into()))?;
    let total = vfs.f_blocks.saturating_mul(vfs.f_frsize);
    let free = vfs.f_bavail.saturating_mul(vfs.f_frsize).min(total);
    // u128: no overflow in the 80% comparisons.
    let over = |used: u64, of: u64| u128::from(used) * 5 > u128::from(of) * 4;
    let warn = over(upload_bytes, quota) || over(total - free, total);
    let i = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
    Ok(DiskUse {
        db_bytes: i(db_bytes),
        upload_bytes: i(upload_bytes),
        quota_bytes: i(quota),
        fs_total: i(total),
        fs_free: i(free),
        warn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use futures_util::StreamExt;

    type Chunk = Result<Bytes, std::io::Error>;

    /// `n` chunks of 1 KiB, one every `every`, then the end.
    fn trickle(n: usize, every: Duration) -> Body {
        Body::from_stream(futures_util::stream::iter(0..n).then(move |_| async move {
            tokio::time::sleep(every).await;
            Chunk::Ok(Bytes::from(vec![7u8; 1024]))
        }))
    }

    async fn read(body: Body, idle_ms: u64, total_ms: u64) -> Result<(u64, Vec<u8>), ApiError> {
        let ms = Duration::from_millis;
        read_body(body, &mut tokio::io::sink(), ms(idle_ms), ms(total_ms)).await
    }

    #[tokio::test]
    async fn read_body_times_out_idle_and_total() {
        let ms = Duration::from_millis;
        // One chunk, then silence: the idle timer ends it.
        let stalled = futures_util::stream::iter([Chunk::Ok(Bytes::from_static(b"x"))])
            .chain(futures_util::stream::pending());
        let t = std::time::Instant::now();
        let e = read(Body::from_stream(stalled), 50, 10_000)
            .await
            .unwrap_err();
        assert_eq!(
            (e.body.code, e.body.message.as_str()),
            (ApiErrorCode::BadRequest, "upload too slow")
        );
        assert!(t.elapsed() < ms(2000), "{:?}", t.elapsed());
        // Steady but endless: never idle, the total timer ends it.
        let t = std::time::Instant::now();
        let e = read(trickle(usize::MAX, ms(10)), 200, 300)
            .await
            .unwrap_err();
        assert_eq!(e.body.message, "upload too slow");
        assert!(
            t.elapsed() >= ms(300) && t.elapsed() < ms(2000),
            "{:?}",
            t.elapsed()
        );
        // A body that keeps moving and ends in time is read whole.
        let (size, head) = read(trickle(5, ms(10)), 200, 5_000).await.unwrap();
        assert_eq!((size, head), (5 * 1024, vec![7u8; 16]));
    }

    #[test]
    fn upload_path_rejects_traversal() {
        let data = FsPath::new("data");
        let valid = "0123456789abcdef0123456789abcdef";
        for bad in [
            "../../vesper.db",
            "%2e%2e",
            "%2e%2e%2f%2e%2e%2fvesper.db",
            &valid[..31],
            &format!("{valid}0"),
            &valid.to_uppercase(),
            "/etc/passwd",
            "0123456789abcdef0123456789abcde\0",
            "0123456789abcdef/../456789abcdef",
            "..23456789abcdef0123456789abcdef",
            "",
        ] {
            assert_eq!(upload_path(data, bad), None, "{bad:?}");
        }
        let p = upload_path(data, valid).unwrap();
        assert_eq!(p, FsPath::new("data/uploads/01").join(valid));
        assert!(p.starts_with("data/uploads/"));
    }

    #[test]
    fn content_disposition_escapes_name() {
        let v = content_disposition("a\"b\r\nSet-Cookie: x.txt", false);
        let s = v.to_str().unwrap();
        assert!(!s.contains('\r') && !s.contains('\n'), "{s}");
        assert!(
            s.starts_with("attachment; filename=\"a_b__Set-Cookie: x.txt\"; "),
            "{s}"
        );
        let star = s.split("filename*=UTF-8''").nth(1).unwrap();
        assert!(star.contains("%22") && star.contains("%0D%0A"), "{star}");
        assert!(!star.contains(['"', ' ', ';', '\\']), "{star}");
        // Unicode keeps its bytes in filename*, the fallback is ASCII.
        let s = content_disposition("café \\ \u{202e}gpj.exe", true);
        let s = s.to_str().unwrap();
        assert!(
            s.starts_with("inline; filename=\"caf_ _ _gpj.exe\"; "),
            "{s}"
        );
        assert!(s.ends_with("caf%C3%A9%20%5C%20%E2%80%AEgpj.exe"), "{s}");
    }

    #[test]
    fn clean_name_rules() {
        assert_eq!(clean_name("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(clean_name("C:\\x\\a.png"), "C:_x_a.png");
        assert_eq!(clean_name("a\r\n\0b\u{202e}c\u{200b}.txt"), "abc.txt");
        assert_eq!(clean_name("  \u{7}  "), "file");
        assert_eq!(clean_name(""), "file");
        assert_eq!(clean_name(&"é".repeat(300)).chars().count(), MAX_NAME);
    }

    #[test]
    fn sniff_rules() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_image(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff_image(b"GIF87a"), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        for no in [
            &b""[..],
            b"\x89PNG",
            b"RIFF\0\0\0\0WAVE",
            b"RIFF",
            b"<svg xmlns=",
            b"<html><script>",
            b"GIF88a",
        ] {
            assert_eq!(sniff_image(no), None, "{no:?}");
        }
    }

    #[test]
    fn sniff_media_rules() {
        for (head, want) in [
            (&b"\0\0\0\x20ftypM4A \0\0\0\0"[..], "audio/mp4"),
            (b"\0\0\0\x20ftypM4B ", "audio/mp4"),
            (b"\0\0\0\x14ftypqt  ", "video/quicktime"),
            (b"\0\0\0\x18ftypisom", "video/mp4"),
            (b"\0\0\0\x18ftypmp42", "video/mp4"),
            (b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81", "video/webm"),
            (b"ID3\x04\0\0\0\0", "audio/mpeg"),
            (b"\xff\xfb\x90\x64", "audio/mpeg"),
            (b"\xff\xf3\x48\xc4", "audio/mpeg"),
            (b"\xff\xe3\x18\xc4", "audio/mpeg"),
            (b"OggS\0\x02", "audio/ogg"),
            (b"RIFF\0\0\0\0WAVEfmt ", "audio/wav"),
            (b"fLaC\0\0\0\x22", "audio/flac"),
        ] {
            assert_eq!(sniff_media(head), Some(want), "{head:?}");
            assert_eq!(sniff(head), Some(want), "{head:?}");
        }
        for no in [
            &b""[..],
            b"\0\0\0\x18ftyp",
            b"\0\0\0\x18ftypheic",
            b"\0\0\0\x1cftypavif",
            b"\0\0\0\x18ftypmif1",
            b"ftypisom",
            b"\x1a\x45\xdf",
            b"RIFF\0\0\0\0WEBPVP8 ",
            b"RIFF\0\0\0\0AVI ",
            b"\xff\xd8\xff\xe0",
            b"\xff\xf1\x50\x80", // AAC ADTS: layer bits 00
            b"\xff\xfb\xf0\x64", // bitrate index 15
            b"\xff\xfb\x9c\x64", // sample rate 3
            b"\xff\xeb\x90\x64", // version 01 is reserved
            b"\xff",
            b"<svg xmlns=",
            b"<html><script>",
        ] {
            assert_eq!(sniff_media(no), None, "{no:?}");
        }
        // jpeg stays an image when both are asked.
        assert_eq!(sniff(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
    }
}
