//! Updates from GitHub releases. A release build asks for the newest release at start and every few hours; when it is
//! newer than this build, the app offers "Update and restart". Nothing is installed without that click. Then the
//! build for this system is downloaded over HTTPS, its SHA-256 is checked against the release's `SHA256SUMS`, it is
//! unpacked beside the app, and the app file is swapped: the running one is renamed to `<name>.old` (kept for a
//! rollback, and Windows can rename a running exe but not overwrite it), the new one takes its name, and the app
//! starts it and quits. From an AppImage the AppImage file is the one replaced.
//!
//! Nothing downloaded is run, or even unpacked, before its hash matches. Debug builds never check.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const LATEST: &str = "https://api.github.com/repos/Toby-Faucher/vesper/releases/latest";
pub const SUMS: &str = "SHA256SUMS";
/// How often a running app asks again.
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The new app gets this argument: it waits for the old one to quit instead of handing over to it.
pub const UPDATED_ARG: &str = "--updated";
/// No build is larger (they are about 30 MB).
const MAX_BYTES: u64 = 500 * 1024 * 1024;

/// A newer release and where its build for this system is.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub version: String,
    pub name: &'static str,
    asset_url: String,
    sums_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// `v2.3.0` or `2.3.0` as numbers. A pre-release (`2.3.0-rc.1`) or anything else is `None`: it is never offered.
pub fn version(s: &str) -> Option<(u64, u64, u64)> {
    let mut n = s.strip_prefix('v').unwrap_or(s).split('.');
    let mut next = || n.next()?.parse::<u64>().ok();
    let v = (next()?, next()?, next()?);
    n.next().is_none().then_some(v)
}

/// The asset with this system's build.
pub fn asset_name(appimage: bool) -> &'static str {
    if cfg!(windows) {
        "vesper-desktop-windows-x64.zip"
    } else if appimage {
        "vesper-desktop-linux-x64.AppImage"
    } else {
        "vesper-desktop-linux-x64.tar.gz"
    }
}

/// The offer in GitHub's answer for `releases/latest`, if it is newer than `current` and has this system's build and
/// the sums. Only https links are taken.
pub fn offer(json: &str, current: &str, appimage: bool) -> Option<Offer> {
    let r: Release = serde_json::from_str(json).ok()?;
    if version(&r.tag_name)? <= version(current)? {
        return None;
    }
    let name = asset_name(appimage);
    let url = |n: &str| {
        r.assets
            .iter()
            .find(|a| a.name == n && a.browser_download_url.starts_with("https://"))
            .map(|a| a.browser_download_url.clone())
    };
    Some(Offer {
        version: r.tag_name.trim_start_matches('v').into(),
        name,
        asset_url: url(name)?,
        sums_url: url(SUMS)?,
    })
}

/// The hash `sha256sum` wrote for `name` (`<hex>  <name>`, or `<hex> *<name>`), lower case.
pub fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hex, file) = l.trim().split_once(char::is_whitespace)?;
        let ok = file.trim_start().trim_start_matches('*') == name
            && hex.len() == 64
            && hex.bytes().all(|b| b.is_ascii_hexdigit());
        ok.then(|| hex.to_ascii_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The file an update replaces: the AppImage when running from one, else this exe.
pub fn target() -> Option<PathBuf> {
    match std::env::var_os("APPIMAGE") {
        Some(a) if cfg!(target_os = "linux") => Some(a.into()),
        _ => std::env::current_exe().ok(),
    }
}

/// Puts `new` in `target`'s place: `target` becomes `<target>.old` (an older `.old` goes), and `new` is moved in. If
/// the last step fails the old file is put back. `new` must be on the same disk (it is unpacked beside `target`).
pub fn swap(target: &Path, new: &Path) -> io::Result<()> {
    let old = with_suffix(target, ".old");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
    }
    match std::fs::remove_file(&old) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    std::fs::rename(target, &old)?;
    if let Err(e) = std::fs::rename(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e);
    }
    Ok(())
}

fn with_suffix(p: &Path, s: &str) -> PathBuf {
    let mut o = p.as_os_str().to_owned();
    o.push(s);
    o.into()
}

fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .https_only(true)
        .user_agent(concat!("vesper-desktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .expect("the HTTP client builds")
}

/// Asks GitHub for the newest release. `None` when there is nothing newer, or no network: said nowhere.
pub async fn check() -> Option<Offer> {
    let res = client()
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let json = res.text().await.ok()?;
    offer(
        &json,
        vesper_protocol::VERSION,
        std::env::var_os("APPIMAGE").is_some(),
    )
}

async fn get(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let res = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| "The download failed.".to_string())?;
    if res.content_length().is_some_and(|n| n > MAX_BYTES) {
        return Err("The download is too large.".into());
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|_| "The download failed.".to_string())?;
    Ok(bytes.to_vec())
}

/// Downloads, checks and swaps in the offer's build. Returns the file to start.
pub async fn install(offer: Offer) -> Result<PathBuf, String> {
    let target = target().ok_or("Vesper could not find its own file.")?;
    let http = client();
    let sums = get(&http, &offer.sums_url).await?;
    let want = sum_for(&String::from_utf8_lossy(&sums), offer.name)
        .ok_or("The release has no checksum for this build.")?;
    let bytes = get(&http, &offer.asset_url).await?;
    if sha256_hex(&bytes) != want {
        return Err("The download did not match its checksum.".into());
    }
    let t = target.clone();
    tokio::task::spawn_blocking(move || unpack_and_swap(&t, offer.name, &bytes))
        .await
        .map_err(|_| "The update could not be installed.".to_string())?
        .map_err(|e| format!("The update could not be installed: {e}."))?;
    Ok(target)
}

/// After the hash matched: unpacks `bytes` in a new folder beside `target` and swaps the app file in.
pub fn unpack_and_swap(target: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let dir = target.parent().ok_or(io::ErrorKind::NotFound)?;
    let work = dir.join(format!(".vesper-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir(&work)?;
    let done = (|| {
        let file = work.join(name);
        std::fs::write(&file, bytes)?;
        let new = if name.ends_with(".AppImage") {
            file
        } else {
            unpack(&file, &work)?;
            let exe = if cfg!(windows) {
                "Vesper.exe"
            } else {
                "vesper-desktop"
            };
            [work.join("Vesper").join(exe), work.join(exe)]
                .into_iter()
                .find(|p| p.is_file())
                .ok_or(io::Error::new(
                    io::ErrorKind::NotFound,
                    "the app is not in the download",
                ))?
        };
        swap(target, &new)
    })();
    let _ = std::fs::remove_dir_all(&work);
    done
}

/// The system's tar: GNU or bsdtar on Linux, bsdtar on Windows 10 and later (which also reads zip files).
fn unpack(archive: &Path, into: &Path) -> io::Result<()> {
    let tar = std::env::var_os("SystemRoot")
        .filter(|_| cfg!(windows))
        .map_or_else(
            || PathBuf::from("tar"),
            |r| Path::new(&r).join(r"System32\tar.exe"),
        );
    let ok = std::process::Command::new(tar)
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()?
        .success();
    if ok {
        Ok(())
    } else {
        Err(io::Error::other("the download could not be unpacked"))
    }
}

/// Starts the new app, which waits for this one to quit.
pub fn restart(target: &Path) -> io::Result<()> {
    std::process::Command::new(target)
        .arg(UPDATED_ARG)
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, names: &[&str]) -> String {
        let assets: Vec<_> = names
            .iter()
            .map(|n| serde_json::json!({"name": n, "browser_download_url": format!("https://github.com/x/{n}")}))
            .collect();
        serde_json::json!({"tag_name": tag, "assets": assets}).to_string()
    }

    #[test]
    fn versions_compare_as_numbers_and_pre_releases_are_never_offered() {
        assert_eq!(version("v2.10.0"), Some((2, 10, 0)));
        assert_eq!(version("2.2.1"), Some((2, 2, 1)));
        assert!(version("v2.10.0") > version("v2.9.9"));
        assert_eq!(version("v2.3.0-rc.1"), None);
        assert_eq!(version("v2.3"), None);
        assert_eq!(version("v2.3.0.1"), None);
        assert_eq!(version("desktop-v2.3.0"), None);
        let all = [asset_name(false), asset_name(true), SUMS];
        assert!(offer(&release("v2.2.0", &all), "2.2.0", false).is_none());
        assert!(offer(&release("v2.1.9", &all), "2.2.0", false).is_none());
        assert!(offer(&release("v3.0.0-rc.1", &all), "2.2.0", false).is_none());
        assert_eq!(
            offer(&release("v2.10.0", &all), "2.9.0", false).map(|o| o.version),
            Some("2.10.0".into())
        );
        assert!(offer("not json", "2.2.0", false).is_none());
    }

    #[test]
    fn the_offer_picks_this_systems_build_and_needs_the_sums() {
        let o = offer(
            &release(
                "v9.0.0",
                &[
                    "vesper-desktop-windows-x64.zip",
                    "vesper-desktop-windows-x64-setup.exe",
                    "vesper-desktop-linux-x64.tar.gz",
                    "vesper-desktop-linux-x64.AppImage",
                    SUMS,
                ],
            ),
            "2.2.0",
            false,
        )
        .unwrap();
        assert_eq!(o.name, asset_name(false));
        assert!(o.asset_url.ends_with(asset_name(false)));
        assert!(o.sums_url.ends_with(SUMS));
        if cfg!(target_os = "linux") {
            assert_eq!(asset_name(false), "vesper-desktop-linux-x64.tar.gz");
            assert_eq!(asset_name(true), "vesper-desktop-linux-x64.AppImage");
        }
        // No sums, or no build for this system: no offer.
        assert!(offer(&release("v9.0.0", &[asset_name(false)]), "2.2.0", false).is_none());
        assert!(offer(&release("v9.0.0", &[SUMS]), "2.2.0", false).is_none());
        // Plain http is never followed.
        let http = release("v9.0.0", &[asset_name(false), SUMS]).replace("https://", "http://");
        assert!(offer(&http, "2.2.0", false).is_none());
    }

    #[test]
    fn the_sum_is_found_by_name_and_must_be_a_sha256() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!(
            "{a}  vesper-desktop-linux-x64.tar.gz\n{b} *vesper-desktop-windows-x64.zip\nxyz  other\n"
        );
        assert_eq!(sum_for(&sums, "vesper-desktop-linux-x64.tar.gz"), Some(a));
        assert_eq!(
            sum_for(&sums, "vesper-desktop-windows-x64.zip"),
            Some("b".repeat(64))
        );
        assert_eq!(sum_for(&sums, "other"), None);
        assert_eq!(sum_for(&sums, "vesper-desktop-linux-x64"), None);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_swap_keeps_the_old_copy_and_unpacks_a_tarball() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("vesper-desktop");
        std::fs::write(&app, b"v1").unwrap();
        let new = dir.path().join("new");
        std::fs::write(&new, b"v2").unwrap();
        swap(&app, &new).unwrap();
        assert_eq!(std::fs::read(&app).unwrap(), b"v2");
        assert_eq!(
            std::fs::read(dir.path().join("vesper-desktop.old")).unwrap(),
            b"v1"
        );
        assert!(!new.exists());
        // A failed swap puts the old file back.
        assert!(swap(&app, &dir.path().join("missing")).is_err());
        assert_eq!(std::fs::read(&app).unwrap(), b"v2");

        // A tarball as the workflow makes it: Vesper/vesper-desktop.
        if cfg!(unix) {
            let src = tempfile::tempdir().unwrap();
            std::fs::create_dir(src.path().join("Vesper")).unwrap();
            std::fs::write(src.path().join("Vesper/vesper-desktop"), b"v3").unwrap();
            let tgz = src.path().join("t.tar.gz");
            let ok = std::process::Command::new("tar")
                .arg("-czf")
                .arg(&tgz)
                .arg("-C")
                .arg(src.path())
                .arg("Vesper")
                .status()
                .unwrap()
                .success();
            assert!(ok);
            let bytes = std::fs::read(tgz).unwrap();
            unpack_and_swap(&app, "vesper-desktop-linux-x64.tar.gz", &bytes).unwrap();
            assert_eq!(std::fs::read(&app).unwrap(), b"v3");
            assert_eq!(
                std::fs::read(dir.path().join("vesper-desktop.old")).unwrap(),
                b"v2"
            );
            // The work folder is gone, and garbage leaves the app as it was.
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
            assert!(unpack_and_swap(&app, "vesper-desktop-linux-x64.tar.gz", b"junk").is_err());
            assert_eq!(std::fs::read(&app).unwrap(), b"v3");
        }
    }
}
