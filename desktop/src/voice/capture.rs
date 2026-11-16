//! The screen for my share, one backend per system, each run on the share's capture thread (`share.rs`):
//!
//! - Windows: Windows Graphics Capture (`windows-capture`) of the screen or window chosen in the share dialog (else
//!   the primary monitor), with the cursor. Windows draws its yellow border round what is captured.
//! - Linux under Wayland: the xdg-desktop-portal ScreenCast (`ashpd`), whose picker chooses the screen or window,
//!   then its PipeWire stream (`pipewire`), with the cursor.
//! - Linux under X11: `GetImage` of the monitor (RandR) or window (`_NET_CLIENT_LIST`) chosen in the share dialog
//!   (else the primary monitor) through `x11rb`, polled at the frame rate, without the cursor.
//!
//! Other systems cannot share.

#[allow(unused_imports)] // the backends of other systems use them
use std::time::Duration;

use super::share::{Screen, Shot};

pub const NOT_HERE: &str = "Screen share is not available on this system.";
pub const FAILED: &str = "Could not share your screen.";
pub const LOST: &str = "The screen capture stopped.";
/// How long `next` waits for a picture before it returns none, so a stop is seen on a still screen.
#[allow(dead_code)]
const WAIT: Duration = Duration::from_millis(100);

/// What to share, chosen in the share dialog from `sources`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// A monitor, by its place in the system's list.
    Monitor(u32),
    /// A window, by the system's id for it.
    Window(u64),
}

/// One line of the share dialog's list.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub source: Source,
    pub name: String,
}

/// The screens, then the windows, to choose from. Empty under Wayland (the system's picker asks) and where none can
/// be listed: the share then takes the primary monitor.
pub fn sources() -> Vec<Choice> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some() {
        return x11::sources().unwrap_or_default();
    }
    #[cfg(target_os = "windows")]
    return windows::sources();
    #[allow(unreachable_code)]
    {
        vec![]
    }
}

/// A screen's line: "Screen 2 (DP-1)", or "Screen 2" with no name.
#[allow(dead_code)]
fn screen_name(i: usize, name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        format!("Screen {}", i + 1)
    } else {
        format!("Screen {} ({name})", i + 1)
    }
}

/// Opens the screen: `source`, or the primary monitor. On Wayland this waits for the system's picker instead; a
/// cancelled picker is an empty error.
pub fn open(fps: u32, source: Option<Source>) -> Result<Box<dyn Screen>, String> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return wayland::open(fps);
        }
        if std::env::var_os("DISPLAY").is_some() {
            return x11::open(fps, source);
        }
    }
    #[cfg(target_os = "windows")]
    return windows::open(fps, source);
    #[allow(unreachable_code)]
    {
        let _ = (fps, source);
        Err(NOT_HERE.into())
    }
}

/// Logs the detail and gives the user a sentence.
#[allow(dead_code)]
fn failed(what: &str, e: impl std::fmt::Display) -> String {
    eprintln!("vesper: screen capture: {what}: {e}");
    FAILED.into()
}

#[cfg(target_os = "linux")]
mod wayland {
    use std::os::fd::OwnedFd;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
    use std::time::Duration;

    use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
    use ashpd::desktop::{PersistMode, Session};
    use pipewire as pw;
    use pw::spa;
    use pw::spa::param::video::VideoFormat;

    use super::{LOST, Screen, Shot, WAIT, failed};

    pub struct Wayland {
        shots: Receiver<Shot>,
        stop: Arc<AtomicBool>,
        session: Option<Session<'static, Screencast<'static>>>,
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        // The portal's D-Bus connection runs on zbus's own thread; this only waits for the answers.
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime builds")
            .block_on(f)
    }

    pub fn open(fps: u32) -> Result<Box<dyn Screen>, String> {
        let (portal, session, node, fd) = block_on(async {
            let portal = Screencast::new().await?;
            let session = portal.create_session().await?;
            portal
                .select_sources(
                    &session,
                    CursorMode::Embedded,
                    SourceType::Monitor | SourceType::Window,
                    false,
                    None,
                    PersistMode::DoNot,
                )
                .await?;
            let streams = portal.start(&session, None).await?.response()?;
            let node = streams.streams().first().map(|s| s.pipe_wire_node_id());
            let fd = portal.open_pipe_wire_remote(&session).await?;
            Ok::<_, ashpd::Error>((portal, session, node, fd))
        })
        .map_err(|e| match e {
            ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => String::new(),
            e => failed("the ScreenCast portal", e),
        })?;
        drop(portal);
        let Some(node) = node else {
            return Err(failed("the ScreenCast portal", "no stream"));
        };
        let (tx, shots) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        std::thread::Builder::new()
            .name("vesper-pipewire".into())
            .spawn(move || {
                if let Err(e) = stream(fd, node, fps, &tx, &s) {
                    eprintln!("vesper: screen capture: PipeWire: {e}");
                }
            })
            .map_err(|e| failed("a thread", e))?;
        Ok(Box::new(Wayland {
            shots,
            stop,
            session: Some(session),
        }))
    }

    impl Screen for Wayland {
        fn next(&mut self) -> Result<Option<Shot>, String> {
            match self.shots.recv_timeout(WAIT) {
                Ok(s) => Ok(Some(s)),
                Err(RecvTimeoutError::Timeout) => Ok(None),
                Err(RecvTimeoutError::Disconnected) => Err(LOST.into()),
            }
        }
    }

    impl Drop for Wayland {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            // The portal's connection is shared, so the session ends only when told; that also ends the stream.
            if let Some(s) = self.session.take() {
                let _ = block_on(s.close());
            }
        }
    }

    /// The picture format PipeWire chose: red, green and blue's bytes in each 4-byte pixel.
    #[derive(Default)]
    struct Format {
        width: u32,
        height: u32,
        rgb: Option<[usize; 3]>,
    }

    /// Runs the PipeWire stream until `stop`, sending each picture.
    fn stream(
        fd: OwnedFd,
        node: u32,
        fps: u32,
        tx: &Sender<Shot>,
        stop: &AtomicBool,
    ) -> Result<(), pw::Error> {
        pw::init();
        let main = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&main, None)?;
        let core = context.connect_fd_rc(fd, None)?;
        let stream = pw::stream::StreamBox::new(
            &core,
            "vesper-share",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )?;
        let tx = tx.clone();
        let _listener = stream
            .add_local_listener_with_user_data(Format::default())
            .param_changed(|_, f, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let mut info = spa::param::video::VideoInfoRaw::default();
                if info.parse(param).is_err() {
                    return;
                }
                *f = Format {
                    width: info.size().width,
                    height: info.size().height,
                    rgb: match info.format() {
                        VideoFormat::BGRx | VideoFormat::BGRA => Some([2, 1, 0]),
                        VideoFormat::RGBx | VideoFormat::RGBA => Some([0, 1, 2]),
                        VideoFormat::xRGB | VideoFormat::ARGB => Some([1, 2, 3]),
                        VideoFormat::xBGR | VideoFormat::ABGR => Some([3, 2, 1]),
                        _ => None,
                    },
                };
            })
            .process(move |stream, f| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let Some(rgb) = f.rgb else { return };
                let Some(d) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let c = d.chunk();
                let (offset, size) = (c.offset() as usize, c.size() as usize);
                let stride = usize::try_from(c.stride()).unwrap_or(0);
                let stride = if stride == 0 {
                    f.width as usize * 4
                } else {
                    stride
                };
                // A chunk of size 0 carries only cursor or damage metadata: no new picture.
                if size == 0 || f.width == 0 || f.height == 0 {
                    return;
                }
                let Some(bytes) = d.data().and_then(|b| b.get(offset..offset + size)) else {
                    return;
                };
                let _ = tx.send(Shot {
                    width: f.width,
                    height: f.height,
                    stride,
                    bpp: 4,
                    rgb,
                    data: bytes.to_vec(),
                });
            })
            .register()?;

        let format = spa::pod::object!(
            spa::utils::SpaTypes::ObjectParamFormat,
            spa::param::ParamType::EnumFormat,
            spa::pod::property!(
                spa::param::format::FormatProperties::MediaType,
                Id,
                spa::param::format::MediaType::Video
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::MediaSubtype,
                Id,
                spa::param::format::MediaSubtype::Raw
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::VideoFormat,
                Choice,
                Enum,
                Id,
                VideoFormat::BGRx,
                VideoFormat::BGRx,
                VideoFormat::BGRA,
                VideoFormat::RGBx,
                VideoFormat::RGBA,
                VideoFormat::xRGB,
                VideoFormat::ARGB,
                VideoFormat::xBGR,
                VideoFormat::ABGR,
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::VideoSize,
                Choice,
                Range,
                Rectangle,
                spa::utils::Rectangle {
                    width: 1920,
                    height: 1080
                },
                spa::utils::Rectangle {
                    width: 1,
                    height: 1
                },
                spa::utils::Rectangle {
                    width: 8192,
                    height: 8192
                }
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::VideoFramerate,
                Choice,
                Range,
                Fraction,
                spa::utils::Fraction { num: fps, denom: 1 },
                spa::utils::Fraction { num: 0, denom: 1 },
                spa::utils::Fraction {
                    num: 1000,
                    denom: 1
                }
            ),
        );
        let bytes = spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &spa::pod::Value::Object(format),
        )
        .map_err(|_| pw::Error::CreationFailed)?
        .0
        .into_inner();
        let mut params = [spa::pod::Pod::from_bytes(&bytes).ok_or(pw::Error::CreationFailed)?];
        stream.connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )?;
        while !stop.load(Ordering::Relaxed) {
            main.loop_().iterate(Duration::from_millis(100));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::time::{Duration, Instant};

    use x11rb::connection::Connection;
    use x11rb::protocol::randr::{ConnectionExt as _, MonitorInfo};
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, ImageFormat};
    use x11rb::rust_connection::RustConnection;

    use super::{Choice, LOST, Screen, Shot, Source, failed, screen_name};

    /// What is captured: an area of the root window (a monitor, or the whole desktop), or one window.
    enum Target {
        Area(i16, i16, u16, u16),
        Window(u32),
    }

    pub struct X11 {
        conn: RustConnection,
        root: u32,
        target: Target,
        every: Duration,
        due: Instant,
    }

    fn monitors(conn: &RustConnection, root: u32) -> Vec<MonitorInfo> {
        conn.randr_get_monitors(root, true)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.monitors)
            .unwrap_or_default()
    }

    fn atom(conn: &RustConnection, name: &str) -> Option<u32> {
        Some(
            conn.intern_atom(false, name.as_bytes())
                .ok()?
                .reply()
                .ok()?
                .atom,
        )
    }

    /// A window's title: `_NET_WM_NAME` (UTF-8), else `WM_NAME`.
    fn title(conn: &RustConnection, w: u32, net_name: u32, utf8: u32) -> String {
        let get = |prop: u32, ty: u32| {
            conn.get_property(false, w, prop, ty, 0, 1024)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.value).trim().to_string())
                .filter(|t| !t.is_empty())
        };
        get(net_name, utf8)
            .or_else(|| get(AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()))
            .unwrap_or_default()
    }

    pub fn sources() -> Option<Vec<Choice>> {
        let (conn, n) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[n].root;
        let mut out = vec![];
        let mut list: Vec<_> = monitors(&conn, root).into_iter().enumerate().collect();
        list.sort_by_key(|(_, m)| !m.primary); // the primary first, the default choice
        for (i, m) in list {
            let name = conn
                .get_atom_name(m.name)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                .unwrap_or_default();
            out.push(Choice {
                source: Source::Monitor(i as u32),
                name: screen_name(i, &name),
            });
        }
        let (Some(list), Some(net_name), Some(utf8), Some(pid)) = (
            atom(&conn, "_NET_CLIENT_LIST"),
            atom(&conn, "_NET_WM_NAME"),
            atom(&conn, "UTF8_STRING"),
            atom(&conn, "_NET_WM_PID"),
        ) else {
            return Some(out);
        };
        let windows: Vec<u32> = conn
            .get_property(false, root, list, AtomEnum::WINDOW, 0, 4096)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect()))
            .unwrap_or_default();
        let me = std::process::id();
        for w in windows {
            let theirs = conn
                .get_property(false, w, pid, AtomEnum::CARDINAL, 0, 1)
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| r.value32().and_then(|mut v| v.next()));
            let name = title(&conn, w, net_name, utf8);
            if theirs == Some(me) || name.is_empty() {
                continue; // my own windows, and ones with no name to show
            }
            out.push(Choice {
                source: Source::Window(u64::from(w)),
                name,
            });
        }
        Some(out)
    }

    pub fn open(fps: u32, source: Option<Source>) -> Result<Box<dyn Screen>, String> {
        let (conn, n) = x11rb::connect(None).map_err(|e| failed("X11", e))?;
        let screen = &conn.setup().roots[n];
        let root = screen.root;
        let whole = Target::Area(0, 0, screen.width_in_pixels, screen.height_in_pixels);
        let target = match source {
            Some(Source::Window(w)) => Target::Window(w as u32),
            // The monitor chosen, else the primary (else the first), not the whole desktop across all monitors.
            _ => {
                let list = monitors(&conn, root);
                let m = match source {
                    Some(Source::Monitor(i)) => list.get(i as usize),
                    _ => list.iter().find(|m| m.primary).or(list.first()),
                };
                m.map_or(whole, |m| Target::Area(m.x, m.y, m.width, m.height))
            }
        };
        Ok(Box::new(X11 {
            conn,
            root,
            target,
            every: Duration::from_secs(1) / fps.max(1),
            due: Instant::now(),
        }))
    }

    impl Screen for X11 {
        fn next(&mut self) -> Result<Option<Shot>, String> {
            std::thread::sleep(self.due.saturating_duration_since(Instant::now()));
            self.due = Instant::now() + self.every;
            let lost = |e: &dyn std::fmt::Display| {
                eprintln!("vesper: screen capture: X11: {e}");
                LOST.to_string()
            };
            let (drawable, x, y, w, h) = match self.target {
                Target::Area(x, y, w, h) => (self.root, x, y, w, h),
                // Asked each time: the window can change size. Gone, the share ends.
                Target::Window(win) => {
                    let g = self
                        .conn
                        .get_geometry(win)
                        .map_err(|e| lost(&e))?
                        .reply()
                        .map_err(|e| lost(&e))?;
                    (win, 0, 0, g.width, g.height)
                }
            };
            let img = match self
                .conn
                .get_image(ImageFormat::Z_PIXMAP, drawable, x, y, w, h, !0)
                .map_err(|e| lost(&e))?
                .reply()
            {
                Ok(img) => img,
                // A minimised (unmapped) window has no picture until it is back.
                Err(_) if matches!(self.target, Target::Window(_)) => return Ok(None),
                Err(e) => return Err(lost(&e)),
            };
            // shortcut: assumes the usual 24 or 32-bit depth in 32-bit little-endian pixels (BGRx); other visuals are rare.
            Ok(Some(Shot {
                width: u32::from(w),
                height: u32::from(h),
                stride: usize::from(w) * 4,
                bpp: 4,
                rgb: [2, 1, 0],
                data: img.data,
            }))
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
    use std::time::Duration;

    use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
        TryIntoCaptureItemWithType,
    };
    use windows_capture::window::Window;

    use super::{Choice, LOST, Screen, Shot, Source, WAIT, failed, screen_name};

    type Error = Box<dyn std::error::Error + Send + Sync>;

    struct Handler(Sender<Shot>);

    impl GraphicsCaptureApiHandler for Handler {
        type Flags = Sender<Shot>;
        type Error = Error;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Handler(ctx.flags))
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let (w, h) = (frame.width(), frame.height());
            let mut buffer = frame.buffer()?;
            let data = buffer.as_nopadding_buffer()?.to_vec();
            let shot = Shot {
                width: w,
                height: h,
                stride: w as usize * 4,
                bpp: 4,
                rgb: [2, 1, 0],
                data,
            };
            if self.0.send(shot).is_err() {
                control.stop();
            }
            Ok(())
        }
    }

    pub struct Windows {
        shots: Receiver<Shot>,
        control: Option<CaptureControl<Handler, Error>>,
    }

    pub fn sources() -> Vec<Choice> {
        let mut out = vec![];
        let primary = Monitor::primary().ok();
        let mut list: Vec<_> = Monitor::enumerate()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .collect();
        list.sort_by_key(|(_, m)| Some(*m) != primary); // the primary first, the default choice
        for (i, m) in list {
            out.push(Choice {
                source: Source::Monitor(i as u32),
                name: screen_name(i, &m.name().unwrap_or_default()),
            });
        }
        // `enumerate` lists only windows that can be captured, none of mine.
        for w in Window::enumerate().unwrap_or_default() {
            let name = w.title().unwrap_or_default();
            if name.trim().is_empty() {
                continue;
            }
            out.push(Choice {
                source: Source::Window(w.as_raw_hwnd() as usize as u64),
                name,
            });
        }
        out
    }

    pub fn open(fps: u32, source: Option<Source>) -> Result<Box<dyn Screen>, String> {
        match source {
            Some(Source::Window(id)) => {
                let w = Window::from_raw_hwnd(id as usize as *mut std::ffi::c_void);
                if !w.is_valid() {
                    return Err(failed("the window", "closed"));
                }
                start(w, fps)
            }
            Some(Source::Monitor(i)) => {
                let m = Monitor::enumerate()
                    .ok()
                    .and_then(|l| l.get(i as usize).copied())
                    .ok_or_else(|| failed("the monitor", "gone"))?;
                start(m, fps)
            }
            None => start(
                Monitor::primary().map_err(|e| failed("the primary monitor", e))?,
                fps,
            ),
        }
    }

    fn start<T: TryIntoCaptureItemWithType + Send + 'static>(
        item: T,
        fps: u32,
    ) -> Result<Box<dyn Screen>, String> {
        let (tx, shots) = channel();
        let settings = Settings::new(
            item,
            CursorCaptureSettings::WithCursor,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(Duration::from_secs(1) / fps.max(1)),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            tx,
        );
        let control = Handler::start_free_threaded(settings)
            .map_err(|e| failed("Windows Graphics Capture", e))?;
        Ok(Box::new(Windows {
            shots,
            control: Some(control),
        }))
    }

    impl Screen for Windows {
        fn next(&mut self) -> Result<Option<Shot>, String> {
            match self.shots.recv_timeout(WAIT) {
                Ok(s) => Ok(Some(s)),
                Err(RecvTimeoutError::Timeout) => match &self.control {
                    Some(c) if c.is_finished() => Err(LOST.into()),
                    _ => Ok(None),
                },
                Err(RecvTimeoutError::Disconnected) => Err(LOST.into()),
            }
        }
    }

    impl Drop for Windows {
        fn drop(&mut self) {
            if let Some(c) = self.control.take() {
                let _ = c.stop();
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Needs a real X server (`DISPLAY`, no `WAYLAND_DISPLAY`), run by hand. Xwayland lists its screens, but its root
    /// window has no picture (`GetImage` fails with BadMatch), so the capture fails there.
    #[test]
    #[ignore]
    fn x11_lists_the_screens_and_captures_the_one_chosen() {
        let list = sources();
        eprintln!("{list:?}");
        let first = list.first().expect("a screen").source;
        assert!(matches!(first, Source::Monitor(_)), "screens come first");
        let mut screen = open(30, Some(first)).unwrap();
        let shot = screen.next().unwrap().expect("a picture");
        assert!(shot.width > 0 && shot.data.len() >= shot.stride * shot.height as usize);
        for c in list
            .iter()
            .filter(|c| matches!(c.source, Source::Window(_)))
        {
            let mut w = open(30, Some(c.source)).unwrap();
            assert!(w.next().is_ok(), "{c:?}");
        }
    }
}
