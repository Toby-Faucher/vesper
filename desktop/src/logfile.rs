//! The log file: `<local data dir>/Vesper/logs/vesper.log`, moved to `vesper.log.1` once it passes 1 MB, so at most
//! about 2 MB stay on disk. Vesper's own lines from info up, other crates' from warn up, and panics.
//!
//! Never log a password, cookie, token, invite code or message text. Names of rooms and people stay out too.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX: u64 = 1024 * 1024;

/// `%LOCALAPPDATA%\Vesper\logs` on Windows, `~/.local/share/Vesper/logs` on Linux.
pub fn dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("Vesper").join("logs"))
}

struct FileLog {
    path: PathBuf,
    file: Mutex<Option<(File, u64)>>,
}

impl FileLog {
    fn open(path: &Path) -> Option<(File, u64)> {
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let len = f.metadata().map_or(0, |m| m.len());
        Some((f, len))
    }

    fn write_line(&self, line: &str) {
        let Ok(mut g) = self.file.lock() else { return };
        if g.as_ref().is_some_and(|(_, len)| *len >= MAX) {
            *g = None; // closed before the rename, which Windows needs
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
        }
        if g.is_none() {
            *g = Self::open(&self.path);
        }
        if let Some((f, len)) = g.as_mut()
            && f.write_all(line.as_bytes()).is_ok()
        {
            *len += line.len() as u64;
        }
    }
}

impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        let ours = m.target().starts_with("vesper");
        m.level()
            <= if ours {
                log::Level::Info
            } else {
                log::Level::Warn
            }
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("{at} {} {}: {}\n", r.level(), r.target(), r.args());
        #[cfg(debug_assertions)]
        eprint!("{line}");
        self.write_line(&line);
    }

    fn flush(&self) {
        if let Ok(mut g) = self.file.lock()
            && let Some((f, _)) = g.as_mut()
        {
            let _ = f.flush();
        }
    }
}

/// Starts the log in `dir` and writes panics to it. Once per process; a second call does nothing.
pub fn start(dir: &Path) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = dir.join("vesper.log");
    let file = Mutex::new(FileLog::open(&path));
    if log::set_boxed_logger(Box::new(FileLog { path, file })).is_err() {
        return;
    }
    log::set_max_level(log::LevelFilter::Info);
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |p| {
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        log::error!(target: "vesper::panic", "thread {thread}: {p}");
        log::logger().flush();
        before(p);
    }));
    log::info!(
        "Vesper {} starting on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
}

/// The last `n` lines of the log, oldest first, taken from the rotated file too when the new one is short.
pub fn tail(dir: &Path, n: usize) -> Vec<String> {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    let (old, new) = (read("vesper.log.1"), read("vesper.log"));
    let lines: Vec<&str> = old.lines().chain(new.lines()).collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_past_the_limit_and_tails_across_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vesper.log");
        let log = FileLog {
            path: path.clone(),
            file: Mutex::new(FileLog::open(&path)),
        };
        let line = format!("{}\n", "x".repeat(1023));
        for _ in 0..1024 {
            log.write_line(&line);
        }
        assert!(!path.with_extension("log.1").exists(), "1 MB exactly");
        log.write_line("last\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "last\n");
        assert_eq!(
            std::fs::metadata(path.with_extension("log.1"))
                .unwrap()
                .len(),
            MAX
        );
        let t = tail(dir.path(), 3);
        assert_eq!(t.len(), 3);
        assert_eq!(t[2], "last");
        assert_eq!(t[1].len(), 1023);
        assert!(tail(dir.path(), 5000).len() == 1025);
    }
}
