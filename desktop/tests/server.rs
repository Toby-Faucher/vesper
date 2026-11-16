//! The desktop client against a real `vesper-server`: sign in, the stored session, live chat, history, reconnect and
//! sign out. No window: these drive `Model` and the network thread, as the views do.
#![cfg(unix)] // the server crates use Unix file permissions; Windows CI runs the unit tests

mod common;

use std::sync::Arc;

use common::{Client, PASSWORD, Server, two};
use vesper_desktop::model::{NOT_REMEMBERED, SESSION_ENDED, Screen};
use vesper_desktop::net::Cmd;
use vesper_desktop::store::{MemStore, Saved};

#[tokio::test(flavor = "multi_thread")]
async fn sign_in_good_and_bad_password_and_an_unreachable_server() {
    let s = Server::start().await;
    s.signup("ann", true).await;
    let mut c = Client::start(Arc::default()).await;
    assert_eq!(c.model.screen, Screen::SignIn);

    let cmds = c.model.sign_in(&s.origin, "ann", "wrong-password-1");
    c.run(cmds);
    c.until("the error", |m| !m.busy).await;
    assert_eq!(c.model.error, "Wrong username or password.");
    assert_eq!(c.model.screen, Screen::SignIn);

    // Nothing listens on port 1.
    let cmds = c.model.sign_in("http://127.0.0.1:1", "ann", PASSWORD);
    c.run(cmds);
    c.until("the error", |m| !m.busy).await;
    assert!(
        c.model.error.starts_with("Could not reach the server"),
        "{}",
        c.model.error
    );

    // An address without a scheme is https, which this plain http server does not speak.
    let cmds = c
        .model
        .sign_in(s.origin.trim_start_matches("http://"), "ann", PASSWORD);
    c.run(cmds);
    c.until("the error", |m| !m.busy).await;
    assert!(!c.model.error.is_empty());

    c.sign_in(&s.origin, "ann").await;
    assert_eq!(c.model.screen, Screen::Main);
    assert_eq!(c.model.server, s.origin);
    let names: Vec<_> = c
        .model
        .state
        .rooms
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, ["general", "hangout"]);
    assert_eq!(c.model.active, Some(c.room("general")));
    let saved = c.store.get().expect("the session is stored");
    assert_eq!(saved.server, s.origin);
    assert!(saved.cookie.starts_with("vesper_session="));
    assert!(!c.model.not_remembered);
}

#[tokio::test(flavor = "multi_thread")]
async fn too_many_bad_passwords_say_rate_limited() {
    let s = Server::start().await;
    s.signup("ann", true).await;
    let mut c = Client::start(Arc::default()).await;
    for _ in 0..10 {
        let cmds = c.model.sign_in(&s.origin, "ann", "wrong-password-1");
        c.run(cmds);
        c.until("the error", |m| !m.busy).await;
        if c.model.error != "Wrong username or password." {
            break;
        }
    }
    assert_eq!(
        c.model.error,
        "Too many tries. Wait a minute and try again."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_is_remembered_then_forgotten_at_sign_out() {
    let s = Server::start().await;
    s.signup("ann", true).await;
    let store = Arc::new(MemStore::default());
    let mut c = Client::start(store.clone()).await;
    c.sign_in(&s.origin, "ann").await;
    let saved = store.get().unwrap();
    drop(c); // the app closes: the network thread ends with its last sender

    // A restart signs in with the stored cookie, no password.
    let mut c = Client::start(store.clone()).await;
    assert_eq!(c.model.screen, Screen::Main);
    c.until("hello", |m| m.online).await;
    assert_eq!(c.model.state.me.as_ref().unwrap().user.username, "ann");

    // Sign out ends the session on the server and forgets the cookie.
    let cmds = c.model.sign_out();
    c.run(cmds);
    assert_eq!(c.model.screen, Screen::SignIn);
    c.until("the store is cleared", |_| store.get().is_none())
        .await;
    let res = s
        .http()
        .get(format!("{}/api/me", s.origin))
        .header("cookie", &*saved.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401, "the old cookie no longer works");

    // A stale stored cookie at start: back to sign in with the address filled, and the store is cleared.
    *store.saved.lock().unwrap() = Some(Saved {
        server: s.origin.clone(),
        cookie: saved.cookie.clone(),
    });
    let c = Client::start(store.clone()).await;
    assert_eq!(c.model.screen, Screen::SignIn);
    assert_eq!(c.model.notice, SESSION_ENDED);
    assert_eq!(c.model.server, s.origin);
    assert!(store.get().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_keyring_sign_in_works_but_is_not_remembered() {
    let s = Server::start().await;
    s.signup("ann", true).await;
    let store = Arc::new(MemStore::broken());
    let mut c = Client::start(store.clone()).await;
    assert!(
        c.model.not_remembered,
        "said at start, when the store cannot be read"
    );
    c.sign_in(&s.origin, "ann").await;
    assert!(c.model.not_remembered);
    assert!(!NOT_REMEMBERED.is_empty());
    drop(c);
    let c = Client::start(store).await;
    assert_eq!(c.model.screen, Screen::SignIn);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_arrives_live_and_counts_as_unread() {
    let (_s, mut a, mut b) = two().await;
    let (general, hangout) = (a.room("general"), a.room("hangout"));
    assert_eq!(b.model.active, Some(general));

    // A room b is not looking at: its unread count rises.
    let cmds = a.model.open_room(hangout);
    a.run(cmds);
    let cmds = a.model.send("hello from ann");
    a.run(cmds);
    a.until("ann's own copy", |m| {
        m.chat.rooms[&hangout]
            .messages
            .iter()
            .any(|m| m.body == "hello from ann")
    })
    .await;
    b.until("the unread count", |m| {
        m.state.unread.get(&hangout).is_some_and(|u| u.unread == 1)
    })
    .await;
    assert!(
        !b.model.chat.rooms.contains_key(&hangout),
        "not loaded until opened"
    );

    // The room b is looking at: the message shows live and is marked read.
    let cmds = a.model.open_room(general);
    a.run(cmds);
    let cmds = a.model.send("in general");
    a.run(cmds);
    b.until("the live message", |m| {
        m.chat.rooms[&general]
            .messages
            .iter()
            .any(|m| m.body == "in general")
    })
    .await;
    let id = b.model.chat.rooms[&general].messages.last().unwrap().id;
    b.until("the read marker", |m| {
        m.state
            .unread
            .get(&general)
            .is_some_and(|u| u.last_read_id == id)
    })
    .await;
    assert_eq!(b.unread(general), 0);

    // Opening the other room loads it and marks it read.
    let cmds = b.model.open_room(hangout);
    b.run(cmds);
    b.until("hangout read", |m| {
        m.state.unread.get(&hangout).is_some_and(|u| u.unread == 0)
    })
    .await;
    assert!(b.has(hangout, "hello from ann"));
    assert_eq!(b.model.chat.rooms[&hangout].messages[0].author_id, a.id());
}

#[tokio::test(flavor = "multi_thread")]
async fn scrolling_up_loads_an_older_page() {
    let s = Server::start().await;
    let ann = s.signup("ann", true).await;
    let general = s.room_id("general").await;
    s.seed(general, &ann.user.user_id, 70).await;
    let mut c = Client::start(Arc::default()).await;
    c.sign_in(&s.origin, "ann").await;
    c.until("the newest page", |m| {
        m.log().is_some_and(|l| !l.messages.is_empty())
    })
    .await;
    let log = c.model.log().unwrap();
    assert_eq!(log.messages.len(), 50);
    assert!(log.more_before);
    assert_eq!(log.messages.last().unwrap().body, "seed 69");

    let cmds = c.model.scrolled(true, false);
    assert!(matches!(cmds.as_slice(), [Cmd::LoadOlder { .. }]));
    c.run(cmds);
    c.until("the older page", |m| !m.loading_older).await;
    let log = c.model.log().unwrap();
    assert_eq!(log.messages.len(), 70);
    assert!(!log.more_before);
    assert_eq!(log.messages[0].body, "seed 0");
    assert!(
        log.messages.windows(2).all(|w| w[0].id < w[1].id),
        "oldest first, no duplicates"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_socket_reconnects_and_resyncs_the_shown_room() {
    let (s, mut a, mut b) = two().await;
    let general = a.room("general");
    b.until("general loaded", |m| m.log().is_some()).await;

    // The server drops bob's socket with a code that allows a reconnect (1012: restarting).
    s.state.hub.close_user(&b.id(), 1012);
    b.until("offline", |m| !m.online).await;

    // Sent while bob is away: he misses the push.
    let cmds = a.model.send("while you were away");
    a.run(cmds);
    a.until("sent", |m| {
        m.chat.rooms[&general]
            .messages
            .iter()
            .any(|m| m.body == "while you were away")
    })
    .await;

    b.until("back online", |m| m.online).await;
    b.until("the missed message, from the refetch", |m| {
        m.chat.rooms[&general]
            .messages
            .iter()
            .any(|m| m.body == "while you were away")
    })
    .await;
    assert_eq!(b.model.active, Some(general));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_ended_elsewhere_goes_back_to_sign_in() {
    let s = Server::start().await;
    s.signup("ann", true).await;
    let store = Arc::new(MemStore::default());
    let mut c = Client::start(store.clone()).await;
    c.sign_in(&s.origin, "ann").await;

    // Another device of ann's revokes this one (Settings > Devices on the web).
    let http = s.http();
    let body = serde_json::json!({ "username": "ann", "password": PASSWORD });
    let res = http
        .post(format!("{}/api/login", s.origin))
        .json(&body)
        .send()
        .await
        .unwrap();
    let other = res.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let devices: serde_json::Value = http
        .get(format!("{}/api/sessions", s.origin))
        .header("cookie", &other)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mine = c.model.state.me.as_ref().unwrap().session_id;
    assert!(devices.as_array().unwrap().iter().any(|d| d["id"] == mine));
    let res = http
        .delete(format!("{}/api/sessions/{mine}", s.origin))
        .header("cookie", &other)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    c.until("sign in", |m| m.screen == Screen::SignIn).await;
    assert_eq!(c.model.notice, SESSION_ENDED);
    c.until("the store is cleared", |_| store.get().is_none())
        .await;
}

/// Settings > Account's picture: a photo cropped by `square_picture` is taken by the server, and the new version
/// reaches the users list (which the avatar cache follows).
#[tokio::test(flavor = "multi_thread")]
async fn a_new_profile_picture_is_taken_and_shown() {
    let s = Server::start().await;
    s.signup("ann", true).await;
    let mut c = Client::start(Arc::default()).await;
    c.sign_in(&s.origin, "ann").await;
    let photo = image::RgbImage::from_fn(1200, 800, |x, y| image::Rgb([x as u8, y as u8, 90]));
    let mut jpeg = std::io::Cursor::new(vec![]);
    photo.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
    let png = vesper_desktop::ui::account::square_picture(jpeg.get_ref()).unwrap();
    c.run(vec![Cmd::PutAvatar(vesper_desktop::net::Png(png))]);
    let me = c.id();
    c.until("my picture", |m| m.state.users[&me].avatar.is_some())
        .await;
}
