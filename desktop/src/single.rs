//! One Vesper at a time. The first one listens on a local socket (a Unix socket in the user's runtime folder, a named
//! pipe on Windows); a second launch finds it, asks it to show its window, and exits. A socket file left by a Vesper
//! that crashed answers nothing: it is removed and taken over.

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

const SHOW: &[u8] = b"show\n";

#[cfg(unix)]
pub use unix::{Listener, claim};

#[cfg(unix)]
mod unix {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    use super::{Claim, SHOW};

    pub struct Listener(UnixListener);

    /// `$XDG_RUNTIME_DIR` (this user's own, mode 0700), else this user's cache folder.
    fn path(name: &str) -> PathBuf {
        dirs::runtime_dir()
            .unwrap_or_else(|| crate::files::cache_dir(""))
            .join(format!("{name}.sock"))
    }

    pub fn claim() -> Claim {
        match super::name() {
            Some(n) => claim_at(&path(n)),
            None => Claim::First(None),
        }
    }

    pub(super) fn claim_at(path: &Path) -> Claim {
        if let Ok(mut s) = UnixStream::connect(path)
            && s.write_all(SHOW).is_ok()
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
        /// Calls `show` on a thread of its own each time another launch asks.
        pub fn serve(self, show: impl Fn() + Send + 'static) {
            std::thread::spawn(move || {
                let Listener(l) = self;
                for mut s in l.incoming().flatten() {
                    let mut buf = [0u8; 8];
                    let n = s.read(&mut buf).unwrap_or(0);
                    if buf[..n] == *SHOW {
                        show();
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

    use super::{Claim, SHOW};

    pub struct Listener(NamedPipeServer, String, tokio::runtime::Runtime);

    pub fn claim() -> Claim {
        let Some(n) = super::name() else {
            return Claim::First(None);
        };
        // One pipe per user: another user's Vesper is never asked.
        let user = std::env::var("USERNAME").unwrap_or_default();
        let pipe = format!(r"\\.\pipe\{n}-{user}");
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
        else {
            return Claim::First(None);
        };
        let asked = rt.block_on(async {
            match ClientOptions::new().open(&pipe) {
                Ok(mut c) => c.write_all(SHOW).await.is_ok(),
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
        /// Calls `show` on a thread of its own each time another launch asks.
        pub fn serve(self, show: impl Fn() + Send + 'static) {
            std::thread::spawn(move || {
                let Listener(mut server, pipe, rt) = self;
                rt.block_on(async move {
                    loop {
                        if server.connect().await.is_err() {
                            return;
                        }
                        let mut buf = [0u8; 8];
                        let n = server.read(&mut buf).await.unwrap_or(0);
                        if buf[..n] == *SHOW {
                            show();
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
        let Claim::First(Some(l)) = unix::claim_at(&path) else {
            panic!("the stale socket is taken over");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        l.serve(move || tx.send(()).unwrap());
        assert!(matches!(unix::claim_at(&path), Claim::Second));
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first was asked to show itself");
    }
}
