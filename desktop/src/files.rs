//! Files from messages on this computer: inline pictures decoded with limits, and safe local names for a file that is
//! saved or opened. Names come from other people: never trusted as a path.

use std::path::{Path, PathBuf};

use crate::timeline::BOX;

/// Images larger than this are not loaded inline; their card still saves them.
pub const IMAGE_MAX: u64 = 20 << 20;

/// A decoded picture, BGRA as GPUI draws it, at most twice `BOX` on each side (sharp on a 2x screen).
#[derive(Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture({}x{})", self.width, self.height)
    }
}

/// Decodes a png, jpeg, gif (first frame) or webp. A picture past 16384 on a side or 256 MB decoded is refused, so a
/// small file cannot ask for a huge buffer.
pub fn decode_picture(bytes: &[u8]) -> Option<Picture> {
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 << 20);
    r.limits(limits);
    let img = r.decode().ok()?;
    // `thumbnail` also scales up: only a large picture is made smaller.
    let (w, h) = (BOX.0 * 2, BOX.1 * 2);
    let img = if img.width() > w || img.height() > h {
        img.thumbnail(w, h)
    } else {
        img
    };
    let img = img.into_rgba8();
    let (width, height) = img.dimensions();
    let mut bgra = img.into_raw();
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Some(Picture {
        width,
        height,
        bgra,
    })
}

/// A name that is safe as one file name on Windows, macOS and Linux: no separators, no characters Windows refuses,
/// no trailing dot or space, not a device name, and never empty.
pub fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().trim_end_matches(['.', ' ']).to_string();
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if s.is_empty() || device {
        format!("file{}", if s.is_empty() { "" } else { "_" }) + &s
    } else {
        s
    }
}

/// The extension a media type is opened with, from the type the server found in the first bytes, so the system
/// player is picked by what the file is, never by the name someone gave it.
pub fn media_ext(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "audio/mp4" => "m4a",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "audio/flac" => "flac",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        _ => return None,
    })
}

/// `dir/name`, or `dir/stem (2).ext` and so on when that is taken. Never an existing file.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = safe_name(name);
    let p = dir.join(&name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("a free name")
}

/// An upload id from the server is used in a URL path: it must be the 32 hex characters the server makes.
pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_on_every_system() {
        assert_eq!(safe_name("notes.txt"), "notes.txt");
        assert_eq!(safe_name("a/b\\c:d*?.txt"), "a_b_c_d__.txt");
        assert_eq!(safe_name("..."), "file");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name(" x. "), "x");
        assert_eq!(safe_name("CON.txt"), "file_CON.txt");
        assert_eq!(safe_name("com1"), "file_com1");
        assert_eq!(safe_name("console.log"), "console.log");
        assert_eq!(safe_name("a\u{0}b"), "a_b");
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.txt"), dir.path().join("a.txt"));
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("a (2).txt"), "x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.txt"),
            dir.path().join("a (3).txt")
        );
        std::fs::write(dir.path().join("b"), "x").unwrap();
        assert_eq!(unique_path(dir.path(), "b"), dir.path().join("b (2)"));
    }

    #[test]
    fn pictures_decode_small_and_bgra_and_junk_is_refused() {
        let mut png = vec![];
        image::RgbaImage::from_pixel(1000, 500, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let p = decode_picture(&png).unwrap();
        assert_eq!((p.width, p.height), (720, 360));
        assert_eq!(&p.bgra[..4], [0, 0, 255, 255], "red as BGRA");
        assert!(decode_picture(b"not a picture").is_none());
        let mut small = vec![];
        image::RgbaImage::from_pixel(30, 20, image::Rgba([0, 0, 0, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut small),
                image::ImageFormat::Png,
            )
            .unwrap();
        let p = decode_picture(&small).unwrap();
        assert_eq!((p.width, p.height), (30, 20), "never scaled up");
        assert!(valid_id("0123456789abcdef0123456789abcdef"));
        assert!(!valid_id("../../api/me"));
        assert_eq!(media_ext("video/mp4"), Some("mp4"));
        assert_eq!(media_ext("application/octet-stream"), None);
    }
}
