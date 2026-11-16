//! Files from messages on this computer: inline pictures decoded with limits, and safe local names for a file that is
//! saved or opened. Names come from other people: never trusted as a path.

use std::path::{Path, PathBuf};

use crate::timeline::BOX;

/// Images larger than this are not loaded inline; their card still saves them.
pub const IMAGE_MAX: u64 = 20 << 20;

/// A decoded picture, BGRA as GPUI draws it, at most twice `BOX` on each side (sharp on a 2x screen). An animated one
/// has its frames one after another in `bgra`, and each frame's time in `delays`; a still one has no delays.
#[derive(Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub delays: Vec<u16>,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture({}x{})", self.width, self.height)
    }
}

/// Decodes a png, jpeg, gif (first frame) or webp. A picture past 16384 on a side or 256 MB decoded is refused, so a
/// small file cannot ask for a huge buffer.
pub fn decode_picture(bytes: &[u8]) -> Option<Picture> {
    decode_within(bytes, (BOX.0 * 2, BOX.1 * 2))
}

/// The image viewer's picture: at most 2048 a side (the viewer is at most 1040 by 640, sharp on a 2x screen).
pub const FULL: (u32, u32) = (2048, 2048);

/// An animated picture's frames together keep at most this many bytes: past it they are made smaller.
const ANIM_BUDGET: u64 = 32 << 20;
/// Frames past this many are left out.
const MAX_FRAMES: usize = 500;

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 << 20);
    limits
}

/// [`decode_picture`] made smaller to fit `max` (w, h) when larger. An animated GIF or WebP keeps its frames.
pub fn decode_within(bytes: &[u8], max: (u32, u32)) -> Option<Picture> {
    if let Some(p) = animated(bytes, max) {
        return Some(p);
    }
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    r.limits(limits());
    let img = r.decode().ok()?.into_rgba8();
    let (w0, h0) = img.dimensions();
    // Only a large picture is made smaller, never a small one larger.
    let s = (max.0 as f64 / w0 as f64).min(max.1 as f64 / h0 as f64);
    let img = if s < 1. {
        let fit = |v: u32| ((v as f64 * s).round() as u32).max(1);
        shrink(&img, fit(w0), fit(h0))
    } else {
        img
    };
    let (width, height) = img.dimensions();
    Some(Picture {
        width,
        height,
        bgra: to_bgra(img),
        delays: vec![],
    })
}

/// `img` made `w` x `h`, its colours averaged weighted by their alpha (premultiplied), so the clear pixels, black
/// with no alpha, do not darken the edges next to them.
fn shrink(img: &image::RgbaImage, w: u32, h: u32) -> image::RgbaImage {
    let mut pre = img.clone();
    premultiply(&mut pre);
    let mut out = image::imageops::thumbnail(&pre, w, h);
    unpremultiply(&mut out);
    out
}

/// Each colour times its alpha: what a resize must average, or clear pixels darken their neighbours.
pub fn premultiply(img: &mut image::RgbaImage) {
    for p in img.pixels_mut() {
        let a = u16::from(p[3]);
        for c in &mut p.0[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
}

/// Back to straight alpha after [`premultiply`].
pub fn unpremultiply(img: &mut image::RgbaImage) {
    for p in img.pixels_mut() {
        let a = u16::from(p[3]);
        for c in &mut p.0[..3] {
            if let Some(v) = (u16::from(*c) * 255 + a / 2).checked_div(a) {
                *c = v.min(255) as u8;
            }
        }
    }
}

/// BGRA as GPUI draws it. A clear pixel takes the colour of the pixels around it (its alpha stays 0): the GPU blends
/// neighbouring pixels when it draws a picture smaller or larger, and GPUI's shader takes straight alpha, so a clear
/// pixel's black would darken the edge.
fn to_bgra(img: image::RgbaImage) -> Vec<u8> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let src = img.into_raw();
    let mut out = src.clone();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if src[i + 3] == 0 {
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for (dx, dy) in [
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                    (-1, 0),
                    (1, 0),
                    (-1, 1),
                    (0, 1),
                    (1, 1),
                ] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let j = ((ny * w + nx) * 4) as usize;
                    let a = u32::from(src[j + 3]);
                    for c in 0..3 {
                        sum[c] += u32::from(src[j + c]) * a;
                    }
                    n += a;
                }
                for c in 0..3 {
                    if let Some(v) = sum[c].checked_div(n) {
                        out[i + c] = v as u8;
                    }
                }
            }
            out.swap(i, i + 2);
        }
    }
    out
}

/// The frames of an animated GIF or WebP, `None` for anything else.
fn frames(bytes: &[u8]) -> Option<image::Frames<'_>> {
    use image::{AnimationDecoder, ImageDecoder};
    let c = std::io::Cursor::new(bytes);
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Gif => {
            let mut d = image::codecs::gif::GifDecoder::new(c).ok()?;
            d.set_limits(limits()).ok()?;
            Some(d.into_frames())
        }
        image::ImageFormat::WebP => {
            let mut d = image::codecs::webp::WebPDecoder::new(c).ok()?;
            if !d.has_animation() {
                return None;
            }
            d.set_limits(limits()).ok()?;
            Some(d.into_frames())
        }
        _ => None,
    }
}

/// An animation of two frames or more: each frame fit to `max`, all of them smaller again when together they pass
/// [`ANIM_BUDGET`]. Read twice (count, then keep), so a long one never holds its frames at full size.
fn animated(bytes: &[u8], max: (u32, u32)) -> Option<Picture> {
    let n = frames(bytes)?
        .take(MAX_FRAMES)
        .take_while(Result::is_ok)
        .count();
    if n < 2 {
        return None;
    }
    let (mut size, mut bgra, mut delays) = (None, vec![], vec![]);
    for f in frames(bytes)?.take(n) {
        let f = f.ok()?;
        let (num, den) = f.delay().numer_denom_ms();
        let ms = num / den.max(1);
        // As browsers do: a frame of 10 ms or less is shown for 100.
        delays.push(if ms <= 10 { 100 } else { ms.min(60_000) as u16 });
        let buf = f.into_buffer();
        let (w, h) = *size.get_or_insert_with(|| anim_size(buf.dimensions(), max, n));
        let buf = if buf.dimensions() == (w, h) {
            buf
        } else {
            shrink(&buf, w, h)
        };
        bgra.extend(to_bgra(buf));
    }
    let (width, height) = size?;
    Some(Picture {
        width,
        height,
        bgra,
        delays,
    })
}

/// The size `n` frames of `(w, h)` are kept at: fit to `max`, then smaller until they fit [`ANIM_BUDGET`].
fn anim_size((w, h): (u32, u32), max: (u32, u32), n: usize) -> (u32, u32) {
    let mut s = (max.0 as f64 / w as f64)
        .min(max.1 as f64 / h as f64)
        .min(1.);
    let bytes = (w as f64 * s) * (h as f64 * s) * 4. * n as f64;
    if bytes > ANIM_BUDGET as f64 {
        s *= (ANIM_BUDGET as f64 / bytes).sqrt();
    }
    (
        ((w as f64 * s) as u32).max(1),
        ((h as f64 * s) as u32).max(1),
    )
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

/// `<cache dir>/Vesper/<sub>`: this user's own folder for media to open (`open`) and pasted pictures (`paste`), never
/// the shared temp folder, where another user could make the folder first or plant a file in it.
pub fn cache_dir(sub: &str) -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Vesper")
        .join(sub)
}

/// Removes pasted pictures a day old: one that was sent is already gone, one never sent is left over.
pub fn clean_pasted() {
    let day = std::time::Duration::from_secs(24 * 3600);
    for e in std::fs::read_dir(cache_dir("paste"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|a| a > day));
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
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

    #[test]
    fn a_gif_keeps_its_frames_within_the_budget() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let frame = |c: u8| {
            let buf = image::RgbaImage::from_pixel(40, 20, image::Rgba([c, 0, 0, 255]));
            image::Frame::from_parts(buf, 0, 0, image::Delay::from_numer_denom_ms(50, 1))
        };
        let mut gif = vec![];
        {
            let mut e = GifEncoder::new(&mut gif);
            e.set_repeat(Repeat::Infinite).unwrap();
            e.encode_frames([frame(255), frame(0), frame(128)]).unwrap();
        }
        let p = decode_within(&gif, (20, 20)).unwrap();
        assert_eq!((p.width, p.height), (20, 10), "fit to the box");
        assert_eq!(p.delays, [50, 50, 50]);
        assert_eq!(p.bgra.len(), 3 * 20 * 10 * 4);
        assert_eq!(&p.bgra[..4], [0, 0, 255, 255], "red as BGRA");
        // 500 frames of 4000 x 4000 would be 32 GB: kept to the budget.
        let (w, h) = anim_size((4000, 4000), (2048, 2048), 500);
        assert!(
            u64::from(w * h) * 4 * 500 <= ANIM_BUDGET && w > 100,
            "{w}x{h}"
        );
        assert_eq!(
            anim_size((100, 50), (720, 560), 10),
            (100, 50),
            "never scaled up"
        );
    }

    #[test]
    fn clear_parts_stay_clear_when_made_smaller() {
        // Left half clear, right half opaque amber: made smaller, the clear side stays clear and the amber keeps its
        // colour (straight alpha, as GPUI's sprite shader takes it).
        let img = image::RgbaImage::from_fn(2000, 1000, |x, _| {
            if x < 1000 {
                image::Rgba([0, 0, 0, 0])
            } else {
                image::Rgba([214, 140, 72, 255])
            }
        });
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let p = decode_picture(&png).unwrap();
        let px = |x: u32, y: u32| {
            let i = ((y * p.width + x) * 4) as usize;
            [p.bgra[i], p.bgra[i + 1], p.bgra[i + 2], p.bgra[i + 3]]
        };
        assert_eq!(px(10, 10)[3], 0, "clear");
        let w = p.width;
        assert_eq!(
            px(w / 2 - 1, 10),
            [72, 140, 214, 0],
            "clear, in the colour next to it"
        );
        assert_eq!(px(p.width - 10, 10), [72, 140, 214, 255], "amber, BGRA");
        let edge = px(p.width / 2, 10);
        assert!(
            edge[3] == 0 || edge[2] >= 200,
            "an edge pixel is not darkened: {edge:?}"
        );
        // Stripes of clear and amber, a pixel each, made smaller: every pixel is part clear, and still amber.
        let img = image::RgbaImage::from_fn(2000, 1000, |x, _| {
            image::Rgba(if x % 2 == 0 {
                [0, 0, 0, 0]
            } else {
                [214, 140, 72, 255]
            })
        });
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let p = decode_picture(&png).unwrap();
        let i = ((10 * p.width + 101) * 4) as usize;
        let mid = &p.bgra[i..i + 4];
        assert!(mid[3] > 60 && mid[3] < 200, "part clear: {mid:?}");
        assert!(mid[2] >= 200, "not darkened by the clear pixels: {mid:?}");
    }
}
