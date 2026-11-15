//! A real `vesper-server` (with its SFU) on a free loopback port, and desktop clients driven through `Model` and the
//! network thread, the same path the GPUI views take. Plain http, so the session cookie has no `Secure` flag; the
//! server turns `Secure` and the `__Host-` name on only for an https origin (`docs/env.md`).
#![allow(dead_code)] // each test binary uses a different part

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use vesper_desktop::model::{Model, Screen};
use vesper_desktop::net::{self, Cmd, Event};
use vesper_desktop::settings::Denoise;
use vesper_desktop::store::MemStore;
use vesper_desktop::voice::{AudioDevices, AudioInput, AudioOutput, RATE};
use vesper_protocol::Me;
use vesper_server::config::Config;
use vesper_server::{AppState, cli};

pub const PASSWORD: &str = "test-password-1";

pub struct Server {
    pub state: AppState,
    /// `http://127.0.0.1:<port>`, the server's `VESPER_ORIGIN`.
    pub origin: String,
    _data: TempDir,
}

impl Server {
    pub async fn start() -> Server {
        let data = TempDir::new().unwrap();
        // The origin must name the port before the server binds it.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let origin = format!("http://127.0.0.1:{port}");
        let cfg = Config {
            http_bind: format!("127.0.0.1:{port}").parse().unwrap(),
            udp_bind: "127.0.0.1:0".parse().unwrap(),
            public_host: None,
            udp_port_public: 0,
            max_room_size: 12,
            ring_timeout: Duration::from_secs(30),
            ping_interval: Duration::from_secs(30),
            max_sockets: 16,
            web_dir: "does-not-exist".into(),
            data_dir: data.path().to_path_buf(),
            origin: origin.clone(),
            secure_cookies: false, // what config.rs sets for an http origin
            upload_quota_bytes: 1 << 30,
            upload_max_bytes: 100 << 20,
            push_subject: None,
            build: "test-build".into(),
            backup_keep: 5,
            backup_every_hours: 0,
            backup_uploads: false,
            header_timeout: vesper_server::config::HEADER_TIMEOUT,
            body_idle: vesper_server::config::BODY_IDLE,
            body_total: vesper_server::config::BODY_TOTAL,
        };
        let sfu = vesper_sfu::Sfu::spawn(vesper_sfu::SfuConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        })
        .unwrap();
        let store = vesper_store::Store::open(&cfg.data_dir.join("vesper.db"))
            .await
            .unwrap();
        let hub = vesper_server::hub::Hub::default();
        let push = vesper_server::push::Push::new(store.clone(), hub.clone(), None)
            .await
            .unwrap();
        let state = AppState {
            push,
            store,
            voice: vesper_server::voice::Voice::new(sfu.clone(), hub.clone(), 12, cfg.ring_timeout),
            passkeys: Arc::new(vesper_server::api_passkeys::Passkeys::new(&cfg.origin)),
            cfg: Arc::new(cfg),
            limits: vesper_server::limits::Limiter::default(),
            hub,
        };
        vesper_server::serve(state.clone(), sfu).await.unwrap();
        Server {
            state,
            origin,
            _data: data,
        }
    }

    /// Signs up `username` with `PASSWORD` over HTTP; the first account is the owner.
    pub async fn signup(&self, username: &str, owner: bool) -> Me {
        let args = [if owner { "create-owner" } else { "invite" }.to_string()];
        let link = cli::run(&args, &self.state.cfg).await.unwrap().unwrap();
        let code = link.split_once('#').unwrap().1;
        let body = json!({ "invite": code, "username": username, "password": PASSWORD, "display_name": username });
        let res = self
            .http()
            .post(format!("{}/api/signup", self.origin))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
        res.json().await.unwrap()
    }

    /// A plain HTTP client that sends our Origin, for what the desktop client cannot do.
    pub fn http(&self) -> reqwest::Client {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(reqwest::header::ORIGIN, self.origin.parse().unwrap());
        reqwest::Client::builder()
            .default_headers(h)
            .build()
            .unwrap()
    }

    pub async fn room_id(&self, name: &str) -> i64 {
        let rooms = self.state.store.rooms().await.unwrap();
        rooms.iter().find(|r| r.name == name).unwrap().id
    }

    /// Posts straight into the store: no push to sockets, no rate limit.
    pub async fn seed(&self, room_id: i64, author_id: &str, n: usize) {
        for i in 0..n {
            let m = vesper_store::NewMessage {
                room_id,
                author_id,
                body: &format!("seed {i}"),
                reply_to: None,
                mentions: &[],
                attachments: &[],
                now: vesper_store::now_ms(),
            };
            self.state.store.post_message(m).await.unwrap();
        }
    }
}

/// Audio for a call without devices: the microphone plays a sine (or silence) in real time, the speakers record.
#[derive(Clone, Default)]
pub struct FakeAudio {
    /// The microphone's tone in Hz; `None` is silence.
    pub tone: Arc<Mutex<Option<f32>>>,
    /// Everything played, 48 kHz mono.
    pub played: Arc<Mutex<Vec<f32>>>,
}

impl FakeAudio {
    pub fn set_tone(&self, hz: Option<f32>) {
        *self.tone.lock().unwrap() = hz;
    }

    /// The last `ms` milliseconds played.
    pub fn last(&self, ms: usize) -> Vec<f32> {
        let p = self.played.lock().unwrap();
        let n = (RATE as usize * ms / 1000).min(p.len());
        p[p.len() - n..].to_vec()
    }
}

struct Mic {
    tone: Arc<Mutex<Option<f32>>>,
    start: Instant,
    made: u64,
    phase: f32,
}

impl AudioInput for Mic {
    fn read(&mut self, out: &mut Vec<f32>) {
        let due = (self.start.elapsed().as_secs_f64() * f64::from(RATE)) as u64;
        let tone = *self.tone.lock().unwrap();
        for _ in self.made..due {
            out.push(tone.map_or(0.0, |hz| {
                self.phase = (self.phase + hz / RATE as f32).fract();
                0.5 * (self.phase * std::f32::consts::TAU).sin()
            }));
        }
        self.made = due;
    }
}

struct Speakers(Arc<Mutex<Vec<f32>>>);

impl AudioOutput for Speakers {
    fn write(&mut self, samples: &[f32]) {
        self.0.lock().unwrap().extend_from_slice(samples);
    }
}

impl AudioDevices for FakeAudio {
    fn input(&self, _: Option<&str>) -> Result<Box<dyn AudioInput>, String> {
        Ok(Box::new(Mic {
            tone: self.tone.clone(),
            start: Instant::now(),
            made: 0,
            phase: 0.0,
        }))
    }

    fn output(&self, _: Option<&str>) -> Result<Box<dyn AudioOutput>, String> {
        Ok(Box::new(Speakers(self.played.clone())))
    }
}

/// A desktop client without a window.
pub struct Client {
    pub tx: UnboundedSender<Cmd>,
    rx: UnboundedReceiver<Event>,
    pub model: Model,
    pub store: Arc<MemStore>,
    pub audio: FakeAudio,
}

impl Client {
    /// Starts the network thread and asks it for the stored session, as the app does at start.
    pub async fn start(store: Arc<MemStore>) -> Client {
        let audio = FakeAudio::default();
        let (tx, rx) = net::start(store.clone(), Arc::new(audio.clone()));
        let mut c = Client {
            tx,
            rx,
            model: Model::default(),
            store,
            audio,
        };
        // The tones must reach the other side as they are, and the test server has no `/dsp/` model.
        c.model.settings.denoise = Denoise::Off;
        c.run(vec![Cmd::Restore]);
        c.until("start", |m| m.screen != Screen::Starting).await;
        c
    }

    pub fn run(&self, cmds: Vec<Cmd>) {
        for c in cmds {
            self.tx.send(c).unwrap();
        }
    }

    /// Feeds events into the model until `done` holds, as the GPUI event pump does.
    pub async fn until(&mut self, what: &str, done: impl Fn(&Model) -> bool) {
        let wait = async {
            while !done(&self.model) {
                // A short wait, so a condition that no event signals (the store) is checked again.
                let Ok(ev) = tokio::time::timeout(Duration::from_millis(50), self.rx.recv()).await
                else {
                    continue;
                };
                let cmds = self.model.apply(ev.expect("the network thread stopped"));
                self.run(cmds);
            }
        };
        if tokio::time::timeout(Duration::from_secs(20), wait)
            .await
            .is_err()
        {
            panic!("timed out waiting for {what}: {:?}", self.model);
        }
    }

    /// Applies the events that came, without waiting.
    pub fn pump(&mut self) {
        self.pump_with(|_| vec![]);
    }

    /// Like `pump`, and `also` sees each event first; what it returns is sent too. For a test's own media.
    pub fn pump_with(&mut self, mut also: impl FnMut(&Event) -> Vec<Cmd>) {
        while let Ok(ev) = self.rx.try_recv() {
            let extra = also(&ev);
            let cmds = self.model.apply(ev);
            self.run(cmds);
            self.run(extra);
        }
    }

    /// Signs in and waits for `hello`.
    pub async fn sign_in(&mut self, server: &str, username: &str) {
        let cmds = self.model.sign_in(server, username, PASSWORD);
        self.run(cmds);
        self.until("signed in and online", |m| m.online).await;
    }

    pub fn id(&self) -> String {
        self.model.state.my_id().to_string()
    }

    pub fn room(&self, name: &str) -> i64 {
        self.model
            .state
            .rooms
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .id
    }

    pub fn has(&self, room: i64, body: &str) -> bool {
        self.model
            .chat
            .rooms
            .get(&room)
            .is_some_and(|l| l.messages.iter().any(|m| m.body == body))
    }

    pub fn unread(&self, room: i64) -> u32 {
        self.model.state.unread.get(&room).map_or(0, |u| u.unread)
    }
}

/// Two signed-in clients: `ann` (owner) and `bob`.
pub async fn two() -> (Server, Client, Client) {
    let s = Server::start().await;
    s.signup("ann", true).await;
    s.signup("bob", false).await;
    let mut a = Client::start(Arc::default()).await;
    a.sign_in(&s.origin, "ann").await;
    let mut b = Client::start(Arc::default()).await;
    b.sign_in(&s.origin, "bob").await;
    (s, a, b)
}

/// Feeds every client's events into its model until `done` holds. A call needs this: the SFU's offers are answered
/// through each client's model.
pub async fn until_all(cs: &mut [&mut Client], what: &str, done: impl Fn(&[&mut Client]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        for c in cs.iter_mut() {
            c.pump();
        }
        if done(cs) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
