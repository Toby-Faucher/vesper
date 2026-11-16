//! One Vesper at a time. The first one listens on a local socket (a Unix socket in the user's runtime folder, a named
//! pipe on Windows); a second launch finds it, asks it to show its window (and open its `vesper://` link, if it was
//! started with one), and exits. A socket file left by a Vesper that crashed answers nothing: it is removed and taken
//! over.
//!
//! The links: `vesper://room/<id>` and `vesper://invite/<code>?server=<address>`. An invite code is a secret: it is
//! held in a [`Secret`], whose `Debug` prints `***`, and never logged.

use vesper_protocol::Secret;

/// What [`claim`] found.
pub enum Claim {
    /// This is the only Vesper: call [`Listener::serve`] once the window is open. `None` when the socket could not be
    /// made: then this one runs alone, without the check.
    First(Option<Listener>),
    /// Another Vesper was running and was asked to show itself: exit.
    Second,
}

/// Debug builds use their own socket, so a build under work never hands over to the installed app. The parity
/// harness runs many at once, so its runs (`VESPER_DEV_SERVER` set) skip the check.
fn name() -> Option<&'static str> {
    if cfg!(debug_assertions) {
        if std::env::var_os("VESPER_DEV_SERVER").is_some() {
            return None;
        }
        Some("vesper-dev")
    } else {
        Some("vesper")
    }
}

/// What a launch asks of the running app: `show\n`, or `open <link>\n`.
fn ask(link: Option<&str>) -> Vec<u8> {
    match link {
        Some(l) => format!("open {l}\n").into_bytes(),
        None => b"show\n".to_vec(),
    }
}

/// The longest message read: a link is far shorter.
const MAX_ASK: usize = 4096;

/// `None` for anything else than [`ask`] writes: nothing happens then. `Some(None)` is a plain show.
fn heard(buf: &[u8]) -> Option<Option<String>> {
    let s = std::str::from_utf8(buf).ok()?.strip_suffix('\n')?;
    if s == "show" {
        return Some(None);
    }
    let l = s.strip_prefix("open ")?;
    link(l)?;
    Some(Some(l.to_string()))
}

/// A `vesper://` link the app opens.
#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    Room(i64),
    /// `server` is the address the invite is for, when the link says.
    Invite {
        code: Secret,
        server: Option<String>,
    },
}

/// The link in a launch's arguments (the system passes it as one), if any.
pub fn link_arg() -> Option<String> {
    std::env::args().skip(1).find(|a| link(a).is_some())
}

/// Reads a `vesper://` link. An invite code is the server's 64 hex characters, as the web's invite page checks.
pub fn link(s: &str) -> Option<Link> {
    if s.len() > 2048 {
        return None;
    }
    let u = url::Url::parse(s).ok()?;
    if u.scheme() != "vesper" {
        return None;
    }
    let path = u.path().trim_matches('/');
    match u.host_str()? {
        "room" => path.parse().ok().filter(|id| *id > 0).map(Link::Room),
        "invite" if path.len() == 64 && path.bytes().all(|b| b.is_ascii_hexdigit()) => {
            let server = u
                .query_pairs()
                .find(|(k, _)| k == "server")
                .map(|(_, v)| v.into_owned())
                .filter(|v| v.starts_with("https://") || v.starts_with("http://"));
            Some(Link::Invite {
                code: path.to_ascii_lowercase().into(),
                server,
            })
        }
        _ => None,
    }
}

#[cfg(unix)]
pub use unix::{Listener, claim};

#[cfg(unix)]
mod unix {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    use super::{Claim, MAX_ASK, ask, heard};

    pub struct Listener(UnixListener);

    /// `$XDG_RUNTIME_DIR` (this user's own, mode 0700), else this user's cache folder.
    fn path(name: &str) -> PathBuf {
        dirs::runtime_dir()
            .unwrap_or_else(|| crate::files::cache_dir(""))
            .join(format!("{name}.sock"))
    }

    pub fn claim(link: Option<&str>) -> Claim {
        match super::name() {
            Some(n) => claim_at(&path(n), link),
            None => Claim::First(None),
        }
    }

    pub(super) fn claim_at(path: &Path, link: Option<&str>) -> Claim {
        if let Ok(mut s) = UnixStream::connect(path)
            && s.write_all(&ask(link)).is_ok()
        {
            return Claim::Second;
        }
        // Nobody answers: no file, or one left by a Vesper that crashed.
        let _ = std::fs::remove_file(path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Claim::First(UnixListener::bind(path).ok().map(Listener))
    }

    impl Listener {
        /// Calls `show` on a thread of its own each time another launch asks, with its link if it had one.
        pub fn serve(self, show: impl Fn(Option<String>) + Send + 'static) {
            std::thread::spawn(move || {
                let Listener(l) = self;
                for s in l.incoming().flatten() {
                    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                    let mut buf = Vec::new();
                    let _ = s.take(MAX_ASK as u64).read_to_end(&mut buf);
                    if let Some(link) = heard(&buf) {
                        show(link);
                    }
                }
            });
        }
    }
}

#[cfg(windows)]
pub use windows::{Listener, claim};

#[cfg(windows)]
mod windows {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    use super::{Claim, MAX_ASK, ask, heard};

    pub struct Listener(NamedPipeServer, String, tokio::runtime::Runtime);

    pub fn claim(link: Option<&str>) -> Claim {
        let Some(n) = super::name() else {
            return Claim::First(None);
        };
        // One pipe per user: another user's Vesper is never asked.
        let user = std::env::var("USERNAME").unwrap_or_default();
        let pipe = format!(r"\\.\pipe\{n}-{user}");
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return Claim::First(None);
        };
        let asked = rt.block_on(async {
            match ClientOptions::new().open(&pipe) {
                Ok(mut c) => c.write_all(&ask(link)).await.is_ok(),
                Err(_) => false,
            }
        });
        if asked {
            return Claim::Second;
        }
        // first_pipe_instance: fails if another process made the pipe first, so none can stand in for Vesper.
        let server = rt.block_on(async {
            ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(&pipe)
        });
        Claim::First(server.ok().map(|s| Listener(s, pipe, rt)))
    }

    impl Listener {
        /// Calls `show` on a thread of its own each time another launch asks, with its link if it had one.
        pub fn serve(self, show: impl Fn(Option<String>) + Send + 'static) {
            std::thread::spawn(move || {
                let Listener(mut server, pipe, rt) = self;
                rt.block_on(async move {
                    loop {
                        if server.connect().await.is_err() {
                            return;
                        }
                        // The client writes one message and closes its end.
                        let mut buf = Vec::new();
                        let read = (&mut server).take(MAX_ASK as u64).read_to_end(&mut buf);
                        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), read).await;
                        if let Some(link) = heard(&buf) {
                            show(link);
                        }
                        // The next launch talks to a new instance of the pipe.
                        server = match ServerOptions::new()
                            .reject_remote_clients(true)
                            .create(&pipe)
                        {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                });
            });
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_asks_the_first_and_a_stale_socket_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.sock");
        // A file left by a Vesper that crashed: nobody listens on it.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let Claim::First(Some(l)) = unix::claim_at(&path, None) else {
            panic!("the stale socket is taken over");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        l.serve(move |link| tx.send(link).unwrap());
        let wait = || rx.recv_timeout(std::time::Duration::from_secs(5));
        assert!(matches!(unix::claim_at(&path, None), Claim::Second));
        assert_eq!(wait().expect("the first was asked to show itself"), None);
        assert!(matches!(
            unix::claim_at(&path, Some("vesper://room/7")),
            Claim::Second
        ));
        assert_eq!(wait().unwrap().as_deref(), Some("vesper://room/7"));
    }

    #[test]
    fn links_are_read_strictly() {
        let code = "ab".repeat(32);
        assert_eq!(link("vesper://room/12"), Some(Link::Room(12)));
        assert_eq!(link("vesper://room/12/"), Some(Link::Room(12)));
        assert_eq!(link("vesper://room/x"), None);
        assert_eq!(link("vesper://room/0"), None);
        assert_eq!(
            link(&format!(
                "vesper://invite/{code}?server=https%3A%2F%2Fv.example"
            )),
            Some(Link::Invite {
                code: code.as_str().into(),
                server: Some("https://v.example".into())
            })
        );
        assert_eq!(
            link(&format!("vesper://invite/{code}?server=file%3A%2F%2Fx")),
            Some(Link::Invite {
                code: code.as_str().into(),
                server: None
            })
        );
        assert_eq!(link("vesper://invite/abc"), None);
        assert_eq!(link(&format!("https://invite/{code}")), None);
        assert_eq!(link("vesper://other/1"), None);
        // The hand-off message: anything else is ignored.
        assert_eq!(heard(&ask(None)), Some(None));
        assert_eq!(
            heard(&ask(Some("vesper://room/3"))),
            Some(Some("vesper://room/3".into()))
        );
        assert_eq!(heard(b"open https://evil.example\n"), None);
        assert_eq!(heard(b"show"), None);
        // The code never shows in a log line.
        let l = link(&format!("vesper://invite/{code}")).unwrap();
        assert!(!format!("{l:?}").contains(&code));
    }
}
