//! The views in GPUI's headless test window, with fake data: no display and no server.

use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use gpui_component::Root;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use vesper_protocol::{Me, MessagePage, ServerMsg};

use super::Vesper;
use crate::model::{HEADPHONES, Phase, Screen};
use crate::net::{Cmd, Event};
use crate::voice::{VoiceCmd, VoiceEvent};
use vesper_protocol::ClientMsg;

fn open(
    cx: &mut TestAppContext,
) -> (
    Entity<Vesper>,
    &mut VisualTestContext,
    UnboundedReceiver<Cmd>,
) {
    cx.update(super::init);
    let (tx, rx) = unbounded_channel();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let v = cx.new(|cx| Vesper::new(tx, window, cx));
        Root::new(v, window, cx)
    });
    let view = root.read_with(cx, |r, _| r.view().clone().downcast::<Vesper>().unwrap());
    (view, cx, rx)
}

fn apply(view: &Entity<Vesper>, cx: &mut VisualTestContext, ev: Event) {
    view.update_in(cx, |v, window, cx| v.apply(ev, window, cx));
    cx.run_until_parked();
}

fn drain(rx: &mut UnboundedReceiver<Cmd>) -> Vec<Cmd> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

fn user(id: &str, name: &str) -> serde_json::Value {
    json!({ "user_id": id, "username": name.to_lowercase(), "display_name": name, "accent": "", "role": "member",
            "banned": false, "bio": "", "avatar": null, "ui_accent": null })
}

fn me() -> Me {
    serde_json::from_value(json!({ "user": user("a", "Ann"), "session_id": 1,
        "server": { "version": "2", "build": "b" }, "upload_max": 1 }))
    .unwrap()
}

fn msg(id: i64, author: &str, body: &str) -> serde_json::Value {
    json!({ "id": id, "room_id": 1, "author_id": author, "body": body, "created_at": 1_700_000_000_000_i64 + id,
            "edited_at": null, "updated_at": id, "deleted": false, "reply_to": null, "reactions": [],
            "attachments": [], "mentions": [], "pinned": false })
}

fn signed_in(view: &Entity<Vesper>, cx: &mut VisualTestContext) {
    apply(
        view,
        cx,
        Event::SignedIn {
            me: Box::new(me()),
            server: "https://v.example".into(),
        },
    );
    let room = |id: i64, kind: &str, name: &str, members: &[&str]| {
        json!({ "kind": kind, "id": id, "name": name, "position": id, "restricted": false, "members": members,
                "archived": false })
    };
    let hello: ServerMsg = serde_json::from_value(json!({ "type": "hello", "me": me(),
        "users": [user("a", "Ann"), user("b", "Bob")],
        "rooms": [room(1, "room", "general", &[]), room(2, "room", "hangout", &[]), room(3, "dm", "x", &["a", "b"])],
        "voice": [{ "user_id": "b", "name": "Bob", "room_id": 2, "muted": false, "deafened": false,
                    "force_muted": false, "share": null }],
        "unread": [{ "room_id": 2, "last_read_id": 0, "unread": 4, "mentions": 1 }] }))
    .unwrap();
    apply(view, cx, Event::Server(Box::new(hello)));
    let mut reply = msg(11, "a", "a reply");
    reply["reply_to"] = json!({ "id": 10, "author_id": "b", "excerpt": "first" });
    reply["attachments"] = json!([{ "id": "u1", "name": "notes.txt", "mime": "text/plain", "size": 3, "kind": "file" }]);
    let mut deleted = msg(12, "b", "");
    deleted["deleted"] = json!(true);
    let page: MessagePage = serde_json::from_value(
        json!({ "messages": [msg(10, "b", "first\nsecond line"), reply,
        deleted], "more_before": false, "more_after": false }),
    )
    .unwrap();
    apply(view, cx, Event::Latest { room: 1, page });
}

#[gpui::test]
fn the_sign_in_view_sends_a_login(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    assert!(matches!(drain(&mut rx).as_slice(), [Cmd::Restore]));
    apply(
        &view,
        cx,
        Event::SignedOut {
            server: Some("https://v.example".into()),
            notice: String::new(),
        },
    );
    assert!(cx.debug_bounds("sign-in-server").is_some());
    assert!(cx.debug_bounds("composer").is_none());

    view.update_in(cx, |v, window, cx| {
        assert_eq!(
            v.server.read(cx).value(),
            "https://v.example",
            "the stored address is filled in"
        );
        v.username
            .update(cx, |i, cx| i.set_value("ann", window, cx));
        v.password
            .update(cx, |i, cx| i.set_value("test-password-1", window, cx));
    });
    let button = cx.debug_bounds("sign-in-button").unwrap();
    cx.simulate_click(button.center(), Modifiers::none());
    cx.run_until_parked();
    let cmds = drain(&mut rx);
    assert!(
        matches!(cmds.as_slice(), [Cmd::Login { server, username, password }]
            if server == "https://v.example" && username == "ann" && &**password == "test-password-1"),
        "{cmds:?}"
    );
    view.read_with(cx, |v, cx| {
        assert!(v.model.busy);
        assert_eq!(
            v.password.read(cx).value(),
            "",
            "the password field is cleared"
        );
    });

    apply(
        &view,
        cx,
        Event::SignInFailed("Wrong username or password.".into()),
    );
    assert!(cx.debug_bounds("sign-in-error").is_some());
}

#[gpui::test]
fn the_room_view_shows_rooms_and_messages_and_sends(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    let cmds = drain(&mut rx);
    assert!(
        cmds.iter().any(|c| matches!(c, Cmd::LoadLatest(1))),
        "{cmds:?}"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::MarkRead { room: 1, id: 12 })),
        "{cmds:?}"
    );
    view.read_with(cx, |v, _| assert_eq!(v.model.screen, Screen::Main));
    for s in [
        "room-1",
        "room-2",
        "room-3",
        "message-10",
        "message-11",
        "message-12",
        "composer",
    ] {
        assert!(cx.debug_bounds(s).is_some(), "{s} is drawn");
    }
    assert!(cx.debug_bounds("sign-in-server").is_none());

    // Enter sends, Shift+Enter makes a new line.
    view.update_in(cx, |v, window, cx| {
        v.composer.update(cx, |i, cx| i.focus(window, cx))
    });
    cx.simulate_input("hello");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("world");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let cmds = drain(&mut rx);
    assert!(
        matches!(cmds.as_slice(), [Cmd::Signal(ClientMsg::Typing { room_id: 1 }), Cmd::Send { room: 1, body, .. }]
            if body == "hello\nworld"),
        "{cmds:?}"
    );
    view.read_with(cx, |v, cx| assert_eq!(v.composer.read(cx).value(), ""));

    // A live message shows at the bottom.
    let m = serde_json::from_value(msg(13, "b", "live")).unwrap();
    apply(
        &view,
        cx,
        Event::Server(Box::new(ServerMsg::MessageNew { message: m })),
    );
    assert!(cx.debug_bounds("message-13").is_some());

    // A click on another room loads it.
    let hangout = cx.debug_bounds("room-2").unwrap();
    cx.simulate_click(hangout.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(
        drain(&mut rx)
            .iter()
            .any(|c| matches!(c, Cmd::LoadLatest(2)))
    );
    view.read_with(cx, |v, _| {
        assert_eq!(v.model.active, Some(2));
        assert!(v.side.gliding(), "the selected fill glides from general");
    });

    // Hide the sidebar: the room's header gets Show sidebar, and the choice is in the settings.
    click(cx, "hide-sidebar");
    view.read_with(cx, |v, _| assert!(v.model.settings.sidebar_collapsed));
    settle(cx);
    click(cx, "show-sidebar");
    view.read_with(cx, |v, _| assert!(!v.model.settings.sidebar_collapsed));
    settle(cx);

    // The account menu: Log out.
    click(cx, "account");
    assert!(cx.debug_bounds("account-menu").is_some());
}

/// Waits out a 220 ms slide and draws once. The test window draws no animation frames, so without this the last
/// frame is the slide's first and the sidebar still covers what is under it. (A caret blink drew a frame by accident.)
fn settle(cx: &mut VisualTestContext) {
    std::thread::sleep(std::time::Duration::from_millis(250));
    cx.update(|w, _| w.refresh());
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let b = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
}

/// The engine is faked: its events are applied by hand.
#[gpui::test]
fn the_call_bar_joins_mutes_deafens_and_leaves(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    drain(&mut rx);
    assert!(cx.debug_bounds("call-bar").is_none());

    click(cx, "join-voice");
    let cmds = drain(&mut rx);
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::Voice(VoiceCmd::Start { call: 1, .. })]
        ),
        "{cmds:?}"
    );
    apply(
        &view,
        cx,
        Event::Voice {
            call: 1,
            ev: VoiceEvent::Offer("offer".into()),
        },
    );
    assert!(matches!(
        drain(&mut rx).as_slice(),
        [Cmd::Signal(ClientMsg::Join { room_id: 1, .. })]
    ));
    // Bob is already in voice in the room shown.
    let bob = serde_json::from_value(
        json!({ "user_id": "b", "name": "Bob", "room_id": 1, "muted": false,
        "deafened": false, "force_muted": false, "share": null }),
    )
    .unwrap();
    let joined = ServerMsg::Joined {
        user_id: "a".into(),
        answer: "answer".into(),
        peers: vec![bob],
    };
    apply(&view, cx, Event::Server(Box::new(joined)));
    apply(
        &view,
        cx,
        Event::Voice {
            call: 1,
            ev: VoiceEvent::Connected,
        },
    );
    apply(
        &view,
        cx,
        Event::Voice {
            call: 1,
            ev: VoiceEvent::Speaking {
                me: false,
                peers: vec!["b".into()],
            },
        },
    );
    apply(
        &view,
        cx,
        Event::Voice {
            call: 1,
            ev: VoiceEvent::Devices("Your microphone was disconnected.".into()),
        },
    );
    drain(&mut rx);
    for s in [
        "call-bar",
        "call-devices",
        "call-mute",
        "call-deafen",
        "call-leave",
        "call-person-a",
        "call-person-b",
        "call-quality",
        "speaking-b",
    ] {
        assert!(cx.debug_bounds(s).is_some(), "{s} is drawn");
    }
    assert!(cx.debug_bounds("speaking-a").is_none());
    // Bob's card from the bar: above his avatar, with his volume.
    click(cx, "call-person-b");
    cx.run_until_parked();
    let avatar = cx.debug_bounds("call-person-b").unwrap();
    let card = cx.debug_bounds("person-card").expect("the card opens");
    assert!(cx.debug_bounds("volume-b").is_some(), "his volume");
    assert!(
        card.bottom() <= avatar.top() && (card.left() - avatar.left()).abs() < gpui::px(1.),
        "above the avatar, left edges lined up: {card:?} {avatar:?}"
    );
    click(cx, "call-person-a");
    assert!(
        cx.debug_bounds("volume-a").is_none(),
        "no slider for myself"
    );
    view.update(cx, |v, cx| v.close_person(cx));
    view.read_with(cx, |v, _| {
        assert_eq!(v.model.call.phase, Phase::Joined);
        assert!(
            !v.model.call.headphones,
            "echo cancellation is on, so no {HEADPHONES}"
        );
    });

    click(cx, "call-mute");
    let cmds = drain(&mut rx);
    assert!(
        matches!(
            cmds.as_slice(),
            [
                Cmd::Voice(VoiceCmd::Muted(true)),
                Cmd::Signal(ClientMsg::SetVoiceState {
                    muted: true,
                    deafened: false
                })
            ]
        ),
        "{cmds:?}"
    );
    click(cx, "call-deafen");
    let cmds = drain(&mut rx);
    assert!(
        matches!(
            cmds.as_slice(),
            [
                Cmd::Voice(VoiceCmd::Deafened(true)),
                Cmd::Signal(ClientMsg::SetVoiceState {
                    muted: true,
                    deafened: true
                })
            ]
        ),
        "{cmds:?}"
    );
    view.read_with(cx, |v, _| {
        assert!(v.model.call.muted && v.model.call.deafened)
    });

    // A force mute: the button cannot unmute.
    apply(
        &view,
        cx,
        Event::Server(Box::new(ServerMsg::ForceMuted { muted: true })),
    );
    drain(&mut rx);
    click(cx, "call-mute");
    assert!(drain(&mut rx).is_empty());

    click(cx, "call-leave");
    let cmds = drain(&mut rx);
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::Signal(ClientMsg::Leave), Cmd::Voice(VoiceCmd::Stop)]
        ),
        "{cmds:?}"
    );
    // GPUI 0.2.2 never forgets a debug selector once drawn, so the bar's going is checked on the model.
    view.read_with(cx, |v, _| assert_eq!(v.model.call.phase, Phase::Idle));

    // A device that does not open: the reason shows, and the app goes on.
    click(cx, "join-voice");
    apply(
        &view,
        cx,
        Event::Voice {
            call: 2,
            ev: VoiceEvent::Failed("No microphone was found.".into()),
        },
    );
    assert!(cx.debug_bounds("call-error").is_some());
    view.read_with(cx, |v, _| {
        assert_eq!(v.model.call.error, "No microphone was found.")
    });
}

/// The share panel with the engine faked: Watch sends the watch, a picture is drawn, the struggle and the end of
/// the share show as text.
#[gpui::test]
fn the_share_panel_watches_and_draws_a_picture(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    click(cx, "join-voice");
    let call = |ev| Event::Voice { call: 1, ev };
    apply(&view, cx, call(VoiceEvent::Offer("offer".into())));
    let bob = serde_json::from_value(
        json!({ "user_id": "b", "name": "Bob", "room_id": 1, "muted": false,
        "deafened": false, "force_muted": false, "share": { "fps": 60, "bitrate_kbps": 8000 } }),
    )
    .unwrap();
    let joined = ServerMsg::Joined {
        user_id: "a".into(),
        answer: "answer".into(),
        peers: vec![bob],
    };
    apply(&view, cx, Event::Server(Box::new(joined)));
    drain(&mut rx);
    assert!(cx.debug_bounds("share-panel").is_some());
    assert!(
        cx.debug_bounds("share-view").is_none(),
        "nothing watched yet"
    );

    click(cx, "share-watch-b");
    let cmds = drain(&mut rx);
    assert!(
        matches!(
            cmds.as_slice(),
            [
                Cmd::Signal(ClientMsg::Watch { user_id: s, on: true }),
                Cmd::Voice(VoiceCmd::Watch { user_id: v, on: true })
            ] if s == "b" && v == "b"
        ),
        "{cmds:?}"
    );
    assert!(cx.debug_bounds("share-view").is_some());
    assert!(
        cx.debug_bounds("share-picture").is_none(),
        "waiting for a picture"
    );

    // A 16x9 red picture: the view takes it from the model and draws it.
    let bgra = [0u8, 0, 255, 255].repeat(16 * 9);
    let frame = crate::voice::Frame {
        user_id: "b".into(),
        width: 16,
        height: 9,
        bgra: std::sync::Arc::new(bgra),
    };
    apply(&view, cx, call(VoiceEvent::Frame(frame)));
    assert!(cx.debug_bounds("share-picture").is_some());
    view.read_with(cx, |v, _| {
        assert!(v.model.call.frame.is_none(), "taken by the view");
        let (id, image) = v.screen.as_ref().unwrap();
        assert_eq!(id, "b");
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (16, 9));
        assert_eq!(
            image.as_bytes(0).unwrap()[..4],
            [0, 0, 255, 255],
            "the bytes stay BGRA"
        );
    });
    let picture = cx.debug_bounds("share-picture").unwrap();
    let panel = cx.debug_bounds("share-view").unwrap();
    assert!(picture.size.width > gpui::px(0.) && picture.size.width <= panel.size.width);

    apply(
        &view,
        cx,
        call(VoiceEvent::Struggling {
            user_id: "b".into(),
            on: true,
        }),
    );
    assert!(cx.debug_bounds("share-struggling").is_some());

    // Bob stops: the picture goes and the panel says so.
    let stop = ServerMsg::PeerShare {
        user_id: "b".into(),
        share: None,
    };
    apply(&view, cx, Event::Server(Box::new(stop)));
    assert!(cx.debug_bounds("share-notice").is_some());
    view.read_with(cx, |v, _| {
        assert!(v.screen.is_none() && v.model.call.watching.is_none());
        assert_eq!(v.model.call.share_notice, "Bob stopped sharing.");
    });
}

/// Settings from the sidebar: each choice lands in the model, push to talk shows its key and takes a new one from a
/// key press, a missing device says so, and nothing is saved without a settings file (the real app's only).
#[gpui::test]
fn the_settings_view_changes_the_settings(cx: &mut TestAppContext) {
    use crate::settings::Denoise;
    use crate::voice::ptt::KeyMsg;

    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    let (keys, watched) = std::sync::mpsc::channel();
    view.update(cx, |v, _| {
        v.ptt_tx = Some(keys);
        v.devices = (vec!["USB mic".into()], vec!["Speakers".into()]);
        v.model.settings.output_device = Some("Old headset".into());
    });
    click(cx, "open-settings");
    for s in [
        "settings",
        "settings-bitrate",
        "settings-mic",
        "settings-speakers",
        "settings-missing",
    ] {
        assert!(cx.debug_bounds(s).is_some(), "{s} is drawn");
    }
    assert!(
        cx.debug_bounds("settings-ptt-key").is_none(),
        "voice activity: no key"
    );
    assert!(
        cx.debug_bounds("settings-mic-1").is_none(),
        "the list is closed"
    );

    // The dropdowns: open, pick, closed again.
    click(cx, "settings-noise");
    click(cx, "settings-noise-1");
    view.read_with(cx, |v, _| {
        assert!(v.page.select.is_none(), "picking closes it")
    });
    click(cx, "settings-echo");
    click(cx, "settings-mic");
    click(cx, "settings-mic-1");
    click(cx, "settings-speakers");
    click(cx, "settings-speakers-0");
    view.read_with(cx, |v, _| {
        let s = &v.model.settings;
        assert_eq!(s.denoise, Denoise::Gate);
        assert!(!s.echo_cancel);
        assert_eq!(s.input_device.as_deref(), Some("USB mic"));
        assert_eq!(s.output_device, None, "back to the default");
    });

    click(cx, "settings-ptt");
    assert!(
        watched.try_iter().all(|m| m == KeyMsg::Watch(None)),
        "no call: the key thread does not poll"
    );
    click(cx, "settings-ptt-key");
    assert!(cx.debug_bounds("settings-capturing").is_some());
    cx.simulate_keystrokes("backspace");
    assert!(cx.debug_bounds("settings-key-problem").is_some());
    cx.simulate_keystrokes("ctrl-v");
    view.read_with(cx, |v, _| {
        assert!(v.capturing_key, "a key with Ctrl is refused")
    });
    cx.simulate_keystrokes("v");
    view.read_with(cx, |v, _| {
        assert_eq!(v.model.settings.ptt_key, "V");
        assert!(v.model.settings.push_to_talk && !v.capturing_key);
    });
    assert!(watched.try_iter().all(|m| m == KeyMsg::Watch(None)));

    // Reset to defaults asks first, then puts Talk mode back (push to talk off), not the devices.
    click(cx, "reset-talk-mode");
    click(cx, "reset-confirm");
    view.read_with(cx, |v, _| {
        let s = &v.model.settings;
        assert!(!s.push_to_talk && s.input_device.is_some());
    });
    click(cx, "settings-ptt");

    // Out of the settings, the window's own V presses and releases push to talk (Wayland has only these).
    click(cx, "settings-close");
    cx.simulate_keystrokes("v");
    // GPUI's test keystrokes are key downs only.
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("v").unwrap(),
    });
    let not_typing = |m: &KeyMsg| !matches!(m, KeyMsg::Typing(_));
    let got: Vec<KeyMsg> = watched.try_iter().filter(not_typing).collect();
    assert_eq!(got, [KeyMsg::Window(true), KeyMsg::Window(false)]);
    cx.simulate_keystrokes("b");
    assert!(watched.try_iter().find(not_typing).is_none(), "another key");

    // The call starts with these settings, closed until the key.
    drain(&mut rx);
    click(cx, "join-voice");
    let cmds = drain(&mut rx);
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::Voice(VoiceCmd::Start { setup, .. }), Cmd::Voice(VoiceCmd::Muted(true))]
                if setup.denoise == Denoise::Gate && setup.input.as_deref() == Some("USB mic")
        ),
        "{cmds:?}"
    );
    assert_eq!(
        watched.try_iter().filter(not_typing).last(),
        Some(KeyMsg::Watch(Some(device_query::Keycode::V))),
        "the call watches the key"
    );
    view.update(cx, |v, cx| v.act(|m| m.set_ptt(true), cx));
    cx.run_until_parked();
    assert!(matches!(
        drain(&mut rx).as_slice(),
        [Cmd::Voice(VoiceCmd::Muted(false))]
    ));
}

/// Push to talk ignores a press while typing: the key thread hears when a text input gains and loses focus.
#[gpui::test]
fn the_key_thread_hears_when_an_input_has_focus(cx: &mut TestAppContext) {
    use crate::voice::ptt::KeyMsg;
    let (view, cx, _rx) = open(cx);
    signed_in(&view, cx);
    let (keys, heard) = std::sync::mpsc::channel();
    view.update(cx, |v, _| v.ptt_tx = Some(keys));
    cx.update(|window, _| window.activate_window()); // GPUI's focus events need an active window
    cx.run_until_parked();
    let typing = |heard: &std::sync::mpsc::Receiver<KeyMsg>| {
        heard
            .try_iter()
            .filter_map(|m| match m {
                KeyMsg::Typing(t) => Some(t),
                _ => None,
            })
            .last()
    };
    view.update_in(cx, |v, window, cx| {
        v.composer.update(cx, |i, cx| i.focus(window, cx))
    });
    cx.run_until_parked();
    assert_eq!(typing(&heard), Some(true), "the composer has focus");
    cx.deactivate_window();
    assert_eq!(
        typing(&heard),
        Some(false),
        "another window has the keyboard"
    );
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert_eq!(typing(&heard), Some(true), "back");
    view.update_in(cx, |v, window, _| v.key_focus.focus(window));
    cx.run_until_parked();
    assert_eq!(typing(&heard), Some(false), "it lost it");
}

// GPUI 0.2.2 never clears `debug_bounds` between frames, so whether something went away is checked in the model.

/// A room with Markdown, a reaction of mine, a link alone, a file and a message of mine to edit.
fn chat_room(view: &Entity<Vesper>, cx: &mut VisualTestContext) {
    signed_in(view, cx);
    let mut md = msg(
        20,
        "b",
        "**bold** and `code` @ann\n- one\n  - two\n> quoted\n```rust\nfn x() {}\n```",
    );
    md["mentions"] = json!(["a"]);
    md["reactions"] = json!([{ "emoji": "\u{1f44d}", "user_ids": ["a", "b"] }]);
    let mut file = msg(22, "b", "notes.txt");
    file["attachments"] = json!([{ "id": "0123456789abcdef0123456789abcdef", "name": "notes.txt",
        "mime": "application/octet-stream", "size": 2048, "kind": "file" }]);
    let mut pic = msg(23, "b", "a picture");
    pic["attachments"] = json!([{ "id": IMAGE_ID, "name": "p.png", "mime": "image/png", "size": 100, "kind": "image" }]);
    let page: MessagePage = serde_json::from_value(json!({ "messages": [msg(10, "b", "first"),
        msg(11, "a", "mine"), md, msg(21, "b", "https://a.test/x"), file, pic],
        "more_before": false, "more_after": false }))
    .unwrap();
    apply(view, cx, Event::Latest { room: 1, page });
}

const IMAGE_ID: &str = "fedcba9876543210fedcba9876543210";

fn tagged(s: String) -> &'static str {
    s.leak()
}

fn focus_composer(view: &Entity<Vesper>, cx: &mut VisualTestContext) {
    view.update_in(cx, |v, window, cx| {
        v.composer.update(cx, |i, cx| i.focus(window, cx))
    });
}

#[gpui::test]
fn markdown_reactions_links_and_files(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    chat_room(&view, cx);
    // An image is fetched once it is drawn, and shows when it comes.
    let cmds = drain(&mut rx);
    let fetches = cmds
        .iter()
        .filter(|c| matches!(c, Cmd::FetchImage(id) if id == IMAGE_ID))
        .count();
    assert_eq!(fetches, 1, "{cmds:?}");
    let picture = crate::files::Picture {
        width: 2,
        height: 2,
        bgra: vec![255; 16],
    };
    apply(
        &view,
        cx,
        Event::Image {
            id: IMAGE_ID.into(),
            picture: Some(picture),
        },
    );
    assert!(
        cx.debug_bounds(tagged(format!("image-{IMAGE_ID}")))
            .is_some()
    );
    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.images.get(IMAGE_ID),
            Some(super::chat::Img::Ready(_))
        ))
    });
    assert!(
        drain(&mut rx)
            .iter()
            .all(|c| !matches!(c, Cmd::FetchImage(_))),
        "fetched once"
    );
    // The Markdown message draws its blocks; the day row heads the first message.
    for s in [
        "message-20",
        "md-20-1",
        "day-10",
        "reactions-20",
        "reaction-20-0",
        "file-0123456789abcdef0123456789abcdef",
    ] {
        assert!(cx.debug_bounds(s).is_some(), "{s} is drawn");
    }
    let blocks = (1..10)
        .filter(|n| cx.debug_bounds(tagged(format!("md-20-{n}"))).is_some())
        .count();
    assert!(blocks >= 4, "paragraph, list items, quote: {blocks}");
    assert!(cx.debug_bounds("day-11").is_none(), "same day");

    // My reaction comes off on a click.
    click(cx, "reaction-20-0");
    assert!(
        matches!(drain(&mut rx).as_slice(), [Cmd::React { id: 20, on: false, emoji }] if emoji == "\u{1f44d}")
    );
    // The picker adds another.
    click(cx, "react-21");
    assert!(cx.debug_bounds("picker-21").is_some());
    click(cx, "pick-21-1");
    assert!(matches!(
        drain(&mut rx).as_slice(),
        [Cmd::React {
            id: 21,
            on: true,
            ..
        }]
    ));
    view.read_with(cx, |v, _| assert_eq!(v.model.picking, None));

    // A link opens in the browser.
    let link = cx.debug_bounds("md-21-1").unwrap();
    cx.simulate_click(
        link.origin + gpui::point(gpui::px(4.), gpui::px(4.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert_eq!(cx.opened_url().as_deref(), Some("https://a.test/x"));

    // Save asks where, then downloads there.
    click(cx, "save-0123456789abcdef0123456789abcdef");
    assert!(cx.did_prompt_for_new_path());
    let to = std::env::temp_dir().join("vesper-test-notes.txt");
    cx.simulate_new_path_selection(|_| Some(to.clone()));
    cx.run_until_parked();
    assert!(
        matches!(drain(&mut rx).as_slice(), [Cmd::Download { to: t, open: false, .. }] if *t == to)
    );

    // A pin, then the pins panel.
    click(cx, "pin-21");
    assert!(matches!(
        drain(&mut rx).as_slice(),
        [Cmd::Pin { id: 21, on: true }]
    ));
    click(cx, "pins-button");
    assert!(matches!(drain(&mut rx).as_slice(), [Cmd::LoadPins(1)]));
    let pinned: vesper_protocol::Message =
        serde_json::from_value(msg(21, "b", "https://a.test/x")).unwrap();
    apply(
        &view,
        cx,
        Event::Pins {
            room: 1,
            pins: Ok(vec![pinned]),
        },
    );
    assert!(cx.debug_bounds("pinned-21").is_some());
}

#[gpui::test]
fn new_messages_rise_a_jump_flashes_and_a_reaction_bumps(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    chat_room(&view, cx);
    view.read_with(cx, |v, _| {
        assert!(v.tl.rising().is_empty(), "history does not rise")
    });
    view.update(cx, |v, _| v.side.unsettle());
    let live = |id: i64| -> Event {
        let m = serde_json::from_value(msg(id, "b", "live")).unwrap();
        Event::Server(Box::new(ServerMsg::MessageNew { message: m }))
    };
    apply(&view, cx, live(30));
    view.read_with(cx, |v, _| {
        assert_eq!(v.tl.rising(), [30], "a live message rises")
    });
    // Mine rises too (my send comes back as the same push).
    let m = serde_json::from_value(msg(31, "a", "mine")).unwrap();
    apply(
        &view,
        cx,
        Event::Server(Box::new(ServerMsg::MessageNew { message: m })),
    );
    view.read_with(cx, |v, _| assert_eq!(v.tl.rising(), [30, 31]));
    // An older page does not.
    let page: MessagePage = serde_json::from_value(json!({ "messages": [msg(5, "b", "old")],
        "more_before": false, "more_after": false }))
    .unwrap();
    apply(&view, cx, Event::Older { room: 1, page });
    view.read_with(cx, |v, _| assert!(!v.tl.rising().contains(&5)));
    // A reply's quote jumps to the message and flashes it.
    view.update(cx, |v, _| assert!(v.model.jump_to(10)));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(v.tl.flashing(), Some(10)));
    // A new reaction bumps its whole pill; a new count bumps the count.
    let mut m = msg(20, "b", "x");
    m["updated_at"] = json!(500);
    m["reactions"] = json!([{ "emoji": "\u{1f44d}", "user_ids": ["a", "b", "c"] },
        { "emoji": "\u{1f602}", "user_ids": ["b"] }]);
    apply(
        &view,
        cx,
        Event::Changed(Box::new(serde_json::from_value(m).unwrap())),
    );
    view.read_with(cx, |v, _| {
        let mut b = v.tl.bumped();
        b.sort();
        assert_eq!(
            b,
            [
                (20, "\u{1f44d}".into(), false),
                (20, "\u{1f602}".into(), true)
            ],
            "the count, then a new pill"
        );
    });
    drain(&mut rx);
}

#[gpui::test]
fn the_new_row_shows_until_read_then_goes(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    // hangout: read up to 1, then two from Bob.
    let page: MessagePage =
        serde_json::from_value(json!({ "messages": [msg(1, "a", "old"), msg(2, "b", "new"),
        msg(3, "b", "newer")], "more_before": false, "more_after": false }))
        .unwrap();
    view.update_in(cx, |v, window, cx| v.open_room(2, window, cx));
    cx.run_until_parked();
    view.update(cx, |v, _| {
        v.model.state.unread.get_mut(&2).unwrap().last_read_id = 1;
    });
    apply(&view, cx, Event::Latest { room: 2, page });
    assert!(cx.debug_bounds("new-row").is_some());
    view.read_with(cx, |v, _| {
        assert!(v.heads[1].new && !v.heads[2].new, "before the first unread");
        assert!(!v.tl.read_done());
    });
    // Read: the row starts its way out.
    view.update(cx, |v, cx| {
        v.model.state.unread.get_mut(&2).unwrap().last_read_id = 3;
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.tl.read_done()));
    drain(&mut rx);
}

#[gpui::test]
fn the_upload_tray_shows_progress_cancels_and_sends(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    chat_room(&view, cx);
    drain(&mut rx);
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    std::fs::write(&a, "aaa").unwrap();
    std::fs::write(&b, "bbbb").unwrap();
    view.update(cx, |v, cx| {
        v.model.state.me.as_mut().unwrap().upload_max = 1 << 20;
        v.attach_paths(vec![a.clone(), b.clone(), dir.path().to_path_buf()], cx)
    });
    cx.run_until_parked();
    let cmds = drain(&mut rx);
    assert!(
        matches!(cmds.as_slice(), [Cmd::Upload { key: 1, path, .. }, Cmd::Upload { key: 2, .. }] if *path == a),
        "the folder is left out: {cmds:?}"
    );
    for s in ["upload-tray", "upload-1", "upload-2", "upload-remove-1"] {
        assert!(cx.debug_bounds(s).is_some(), "{s} is drawn");
    }
    apply(
        &view,
        cx,
        Event::UploadProgress {
            key: 1,
            progress: 0.5,
        },
    );
    view.read_with(cx, |v, _| assert_eq!(v.model.tray.items[0].progress, 0.5));

    // Cancel stops the running upload and the row goes.
    click(cx, "upload-remove-2");
    assert!(matches!(drain(&mut rx).as_slice(), [Cmd::CancelUpload(2)]));
    view.read_with(cx, |v, _| assert_eq!(v.model.tray.items.len(), 1));

    // Sending waits for the file, then takes it.
    focus_composer(&view, cx);
    cx.simulate_keystrokes("enter");
    assert!(cx.debug_bounds("room-error").is_some(), "still uploading");
    let att = serde_json::from_value(json!({ "id": "u1", "name": "a.txt",
        "mime": "application/octet-stream", "size": 3, "kind": "file" }))
    .unwrap();
    apply(
        &view,
        cx,
        Event::Uploaded {
            key: 1,
            attachment: att,
        },
    );
    drain(&mut rx);
    cx.simulate_keystrokes("enter");
    let cmds = drain(&mut rx);
    assert!(
        matches!(cmds.as_slice(), [Cmd::Send { body, attachments, .. }] if body == "a.txt" && attachments.len() == 1),
        "{cmds:?}"
    );
}

#[gpui::test]
fn edit_reply_delete_mentions_and_typing(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    chat_room(&view, cx);
    drain(&mut rx);

    // Up in the empty composer edits my last message; Enter saves.
    focus_composer(&view, cx);
    cx.simulate_keystrokes("up");
    assert!(cx.debug_bounds("edit-box").is_some());
    view.read_with(cx, |v, cx| {
        assert_eq!(v.model.editing, Some(11));
        assert_eq!(v.edit.read(cx).value(), "mine");
    });
    cx.simulate_input(" again");
    cx.simulate_keystrokes("enter");
    let cmds = drain(&mut rx);
    assert!(
        matches!(cmds.as_slice(), [Cmd::Edit { id: 11, body }] if body == "mine again"),
        "{cmds:?}"
    );
    let mut edited = msg(11, "a", "mine again");
    (edited["updated_at"], edited["edited_at"]) = (json!(99), json!(99));
    apply(
        &view,
        cx,
        Event::Edited(Box::new(serde_json::from_value(edited).unwrap())),
    );
    view.read_with(cx, |v, _| assert_eq!(v.model.editing, None));
    // Esc leaves an edit without saving.
    cx.simulate_keystrokes("up");
    view.read_with(cx, |v, _| assert_eq!(v.model.editing, Some(11)));
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert_eq!(v.model.editing, None));
    assert!(drain(&mut rx).is_empty());

    // Delete is in the More menu, and asks first.
    click(cx, "more-11");
    assert!(cx.debug_bounds("menu-11").is_some());
    click(cx, "delete-11");
    assert!(cx.debug_bounds("confirm-11").is_some());
    assert!(drain(&mut rx).is_empty());
    click(cx, "confirm-delete");
    assert!(matches!(drain(&mut rx).as_slice(), [Cmd::Delete(11)]));
    assert!(
        cx.debug_bounds("delete-10").is_none(),
        "not someone else's, for a member"
    );

    // Reply, with the @ list on the way.
    click(cx, "reply-10");
    assert!(cx.debug_bounds("reply-bar").is_some());
    focus_composer(&view, cx);
    cx.simulate_input("hi @b");
    assert!(cx.debug_bounds("mention-bob").is_some());
    cx.simulate_keystrokes("tab");
    view.read_with(cx, |v, cx| {
        assert_eq!(v.composer.read(cx).value(), "hi @bob ")
    });
    view.read_with(cx, |v, _| assert!(v.mention.is_none()));
    drain(&mut rx);
    cx.simulate_keystrokes("enter");
    assert!(
        matches!(drain(&mut rx).as_slice(), [Cmd::Send { body, reply_to: Some(10), .. }] if body == "hi @bob")
    );

    // Someone typing shows under the list.
    apply(
        &view,
        cx,
        Event::Server(Box::new(ServerMsg::Typing {
            room_id: 1,
            user_id: "b".into(),
        })),
    );
    assert!(cx.debug_bounds("typing").is_some());
}

/// A name in the sidebar's voice list opens the person card to its right; a moderator's card acts on a member, and
/// Disconnect asks first.
#[gpui::test]
fn the_person_card_opens_from_the_sidebar_and_moderates(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    view.update(cx, |v, _| {
        v.model.state.me.as_mut().unwrap().user.role = vesper_protocol::Role::Moderator;
    });
    drain(&mut rx);
    click(cx, "person-2-b");
    let row = cx.debug_bounds("person-2-b").unwrap();
    let card = cx.debug_bounds("person-card").expect("the card opens");
    assert!(
        (card.left() - row.right() - gpui::px(4.)).abs() < gpui::px(1.)
            && (card.top() - row.top()).abs() < gpui::px(1.),
        "4 px right of the row, tops lined up: {card:?} {row:?}"
    );
    click(cx, "person-force-mute");
    let cmds = drain(&mut rx);
    assert!(
        matches!(cmds.as_slice(), [Cmd::Moderate { user_id, action: "force-mute", muted: Some(true) }] if user_id == "b"),
        "{cmds:?}"
    );
    apply(&view, cx, Event::Moderated(Err("Not allowed.".into())));
    assert!(cx.debug_bounds("person-error").is_some());
    click(cx, "person-disconnect");
    assert!(cx.debug_bounds("person-confirm").is_some());
    click(cx, "person-confirm-disconnect");
    let cmds = drain(&mut rx);
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::Moderate {
                action: "disconnect",
                muted: None,
                ..
            }]
        ),
        "{cmds:?}"
    );

    apply(&view, cx, Event::Moderated(Ok(())));
    // Message opens the DM the server answers with.
    click(cx, "person-2-b");
    click(cx, "person-message");
    assert!(matches!(drain(&mut rx).as_slice(), [Cmd::OpenDm(u)] if u == "b"));
    let dm = serde_json::from_value(json!({ "kind": "dm", "id": 3, "name": "x", "position": 3,
        "restricted": false, "members": ["a", "b"], "archived": false }))
    .unwrap();
    apply(&view, cx, Event::DmOpened(Box::new(dm)));
    view.read_with(cx, |v, _| {
        assert_eq!(v.model.active, Some(3));
        assert!(v.person.is_none());
    });
}

/// A name in the timeline opens its author's card.
#[gpui::test]
fn a_name_in_the_timeline_opens_the_card(cx: &mut TestAppContext) {
    let (view, cx, _rx) = open(cx);
    signed_in(&view, cx);
    // The list settles its alignment a frame after the page: draw again, so the bounds are the settled ones.
    cx.update(|w, _| w.refresh());
    cx.run_until_parked();
    click(cx, "name-10");
    view.read_with(cx, |v, _| {
        assert_eq!(v.person.as_ref().map(|c| c.user_id.as_str()), Some("b"))
    });
    click(cx, "name-10");
    view.read_with(cx, |v, _| {
        assert!(v.person.is_none(), "a second click closes it")
    });
}

/// A DM call rings: Accept joins it, Decline tells the server.
#[gpui::test]
fn a_ring_is_accepted_or_declined(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    let ring = |room| ServerMsg::RingStart {
        room_id: room,
        user_id: "b".into(),
    };
    apply(&view, cx, Event::Server(Box::new(ring(3))));
    assert!(cx.debug_bounds("ring-prompt").is_some());
    drain(&mut rx);
    click(cx, "ring-decline");
    assert!(matches!(
        drain(&mut rx).as_slice(),
        [Cmd::Signal(ClientMsg::DeclineRing { room_id: 3 })]
    ));
    apply(&view, cx, Event::Server(Box::new(ring(3))));
    click(cx, "ring-accept");
    let cmds = drain(&mut rx);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::Voice(VoiceCmd::Start { .. }))),
        "{cmds:?}"
    );
    view.read_with(cx, |v, _| {
        assert_eq!(v.model.active, Some(3));
        assert!(v.model.rings.is_empty());
    });
}

/// Offline: the banner counts down and Try now asks the socket to try at once.
#[gpui::test]
fn the_connection_banner_retries_now(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    drain(&mut rx);
    apply(
        &view,
        cx,
        Event::Offline {
            retry_in: std::time::Duration::from_secs(4),
        },
    );
    assert!(cx.debug_bounds("connection-banner").is_some());
    click(cx, "retry-now");
    assert!(matches!(drain(&mut rx).as_slice(), [Cmd::RetryNow]));
}

// Idle means no redraws: nothing the user can see moves, so no view or input asks to be drawn again. A caret blink
// used to keep going on inputs that were not focused (vendor/gpui-component/PATCHED.txt): the password cleared after
// sign in, and the composer focused by a room that opened and lost the focus before it was drawn.
#[gpui::test]
fn an_idle_window_asks_for_no_redraws(cx: &mut TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    view.update_in(cx, |v, window, cx| {
        v.password.update(cx, |i, cx| i.set_value("", window, cx));
        v.open_room(2, window, cx);
        v.key_focus.focus(window);
    });
    // Past every animation (the longest is the 1.5 s sidebar calm).
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
    let notes = Rc::new(Cell::new(0));
    let watched = view.read_with(cx, |v, _| {
        [&v.server, &v.username, &v.password, &v.composer, &v.edit].map(|e| e.clone())
    });
    cx.update(|_, cx| {
        let n = notes.clone();
        cx.observe(&view, move |_, _| n.set(n.get() + 1)).detach();
        for e in watched {
            let n = notes.clone();
            cx.observe(&e, move |_, _| n.set(n.get() + 1)).detach();
        }
    });
    // The caret's blink runs on smol's real-time timer, not the test clock: wait for real, past two blinks.
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(450));
        cx.run_until_parked();
    }
    assert_eq!(notes.get(), 0, "redraws asked for while idle");
    drain(&mut rx);
}

fn new_msg(view: &Entity<Vesper>, cx: &mut VisualTestContext, id: i64) {
    let m = serde_json::from_value(msg(id, "b", "live")).unwrap();
    apply(
        view,
        cx,
        Event::Server(Box::new(ServerMsg::MessageNew { message: m })),
    );
}

fn composer_text(view: &Entity<Vesper>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, cx| v.composer.read(cx).value().to_string())
}

/// A dialog takes the focus: Enter confirms it (not the composer under it), and a click on its backdrop reaches
/// nothing under it.
#[gpui::test]
fn a_dialog_takes_enter_and_blocks_clicks(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    chat_room(&view, cx);
    focus_composer(&view, cx);
    cx.simulate_input("half typed");
    click(cx, "more-11");
    click(cx, "delete-11");
    assert!(cx.debug_bounds("confirm-11").is_some());
    drain(&mut rx);
    // The sidebar's room 2 under the backdrop.
    click(cx, "room-2");
    view.read_with(cx, |v, _| {
        assert_eq!(v.model.active, Some(1), "the backdrop took the click");
        assert_eq!(v.model.confirm_delete, Some(11));
    });
    cx.simulate_keystrokes("enter");
    let cmds = drain(&mut rx);
    assert!(matches!(cmds.as_slice(), [Cmd::Delete(11)]), "{cmds:?}");
    assert_eq!(composer_text(&view, cx), "half typed");
    view.read_with(cx, |v, _| assert_eq!(v.model.confirm_delete, None));
    // The focus is back in the composer.
    cx.simulate_input("!");
    assert_eq!(composer_text(&view, cx), "half typed!");
}

/// Esc closes the topmost thing, like the web: a dialog, the person card, a menu, then declines a ring.
#[gpui::test]
fn escape_closes_dialogs_cards_menus_and_declines_a_ring(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    chat_room(&view, cx);
    apply(
        &view,
        cx,
        Event::Server(Box::new(ServerMsg::RingStart {
            room_id: 3,
            user_id: "b".into(),
        })),
    );
    focus_composer(&view, cx);
    click(cx, "more-11");
    click(cx, "delete-11");
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert_eq!(v.model.confirm_delete, None));
    assert!(drain(&mut rx).iter().all(|c| !matches!(c, Cmd::Delete(_))));

    focus_composer(&view, cx);
    click(cx, "person-2-b");
    view.read_with(cx, |v, _| assert!(v.person.is_some()));
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert!(v.person.is_none()));

    click(cx, "more-11");
    view.read_with(cx, |v, _| assert_eq!(v.tl.menu, Some(11)));
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert_eq!(v.tl.menu, None));

    click(cx, "account");
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert!(!v.side.account_open));

    drain(&mut rx);
    cx.simulate_keystrokes("escape");
    assert!(matches!(
        drain(&mut rx).as_slice(),
        [Cmd::Signal(ClientMsg::DeclineRing { room_id: 3 })]
    ));
}

/// Joins room 1's call with Bob sharing, watches him and takes a picture.
fn watching_bob(
    view: &Entity<Vesper>,
    cx: &mut VisualTestContext,
    rx: &mut UnboundedReceiver<Cmd>,
) {
    signed_in(view, cx);
    click(cx, "join-voice");
    let call = |ev| Event::Voice { call: 1, ev };
    apply(view, cx, call(VoiceEvent::Offer("offer".into())));
    let bob = serde_json::from_value(
        json!({ "user_id": "b", "name": "Bob", "room_id": 1, "muted": false,
        "deafened": false, "force_muted": false, "share": { "fps": 60, "bitrate_kbps": 8000 } }),
    )
    .unwrap();
    let joined = ServerMsg::Joined {
        user_id: "a".into(),
        answer: "answer".into(),
        peers: vec![bob],
    };
    apply(view, cx, Event::Server(Box::new(joined)));
    click(cx, "share-watch-b");
    drain(rx);
}

fn frame() -> crate::voice::Frame {
    crate::voice::Frame {
        user_id: "b".into(),
        width: 16,
        height: 9,
        bgra: std::sync::Arc::new([0u8, 0, 255, 255].repeat(16 * 9)),
    }
}

/// The share counts as not shown, and the timer for its `watch off` runs (the grace is on the real clock, which
/// the model's tests cover).
fn assert_off_due(view: &Entity<Vesper>, cx: &mut VisualTestContext) {
    view.read_with(cx, |v, _| {
        assert!(!v.model.call.share_visible);
        assert!(v.watch_timer.is_some());
    });
}

/// A window that is not drawn (minimised, covered) stops watching: pictures that wait untaken for 2 s say so, and
/// the `watch off` is due after the grace, with no draw.
#[gpui::test]
fn a_share_nobody_draws_is_stopped(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    watching_bob(&view, cx, &mut rx);
    // A picture comes, and no frame is drawn (the model is fed without a notify, as a hidden window would).
    view.update(cx, |v, cx| {
        v.model.call.frame = Some(frame());
        v.frame_came(cx);
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
    assert_off_due(&view, cx);
}

/// Closing the window during a call minimises it: the share counts as not shown at once.
#[gpui::test]
fn a_minimised_window_stops_watching(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    watching_bob(&view, cx, &mut rx);
    view.update(cx, |v, cx| v.minimised_by_close(cx));
    cx.update(|w, _| w.refresh());
    cx.run_until_parked();
    assert_off_due(&view, cx);
}

/// Each room keeps its own draft, like the web: text typed in one never goes to another.
#[gpui::test]
fn each_room_keeps_its_draft(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    focus_composer(&view, cx);
    cx.simulate_input("for general");
    click(cx, "room-2");
    assert_eq!(composer_text(&view, cx), "");
    cx.simulate_input("for hangout");
    click(cx, "room-1");
    assert_eq!(composer_text(&view, cx), "for general");
    drain(&mut rx);
    click(cx, "room-2");
    assert_eq!(composer_text(&view, cx), "for hangout");
    assert!(
        !drain(&mut rx)
            .iter()
            .any(|c| matches!(c, Cmd::Signal(ClientMsg::Typing { .. }))),
        "a draft put back is not typing"
    );
}

/// Settings covers the room: what comes in the room meanwhile is not marked read until it shows again.
#[gpui::test]
fn the_room_behind_settings_is_not_marked_read(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    view.update(cx, |v, _| v.model.focused = true);
    click(cx, "open-settings");
    drain(&mut rx);
    new_msg(&view, cx, 13);
    let cmds = drain(&mut rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, Cmd::MarkRead { .. })),
        "{cmds:?}"
    );
    click(cx, "settings-close");
    assert!(
        drain(&mut rx)
            .iter()
            .any(|c| matches!(c, Cmd::MarkRead { room: 1, id: 13 }))
    );
}

/// A ring plays its sound every 2.5 s while it shows, like the web, minimised or not.
#[gpui::test]
fn a_ring_plays_until_it_ends(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    let ring = ServerMsg::RingStart {
        room_id: 3,
        user_id: "b".into(),
    };
    apply(&view, cx, Event::Server(Box::new(ring)));
    view.read_with(cx, |v, _| {
        assert_eq!(v.ring_sound.as_ref().map(|r| r.0), Some(3))
    });
    apply(
        &view,
        cx,
        Event::Server(Box::new(ServerMsg::RingStop { room_id: 3 })),
    );
    view.read_with(cx, |v, _| assert!(v.ring_sound.is_none()));
    drain(&mut rx);
}

/// Late answers after a sign out change nothing: a picture is not cached, a download is not opened.
#[gpui::test]
fn late_pictures_after_sign_out_are_dropped(cx: &mut TestAppContext) {
    let (view, cx, _rx) = open(cx);
    signed_in(&view, cx);
    view.update_in(cx, |v, window, cx| v.sign_out(window, cx));
    let picture = crate::files::Picture {
        width: 2,
        height: 2,
        bgra: vec![255; 16],
    };
    apply(
        &view,
        cx,
        Event::Image {
            id: IMAGE_ID.into(),
            picture: Some(picture),
        },
    );
    view.read_with(cx, |v, _| assert!(v.images.is_empty()));
}

/// Enter while an input method is composing ends the composition; it does not send.
#[gpui::test]
fn enter_while_composing_does_not_send(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    focus_composer(&view, cx);
    view.update_in(cx, |v, window, cx| {
        v.composer.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "ka", None, window, cx)
        })
    });
    drain(&mut rx);
    cx.simulate_keystrokes("enter");
    assert!(
        !drain(&mut rx).iter().any(|c| matches!(c, Cmd::Send { .. })),
        "nothing sent"
    );
    assert_eq!(composer_text(&view, cx), "ka");
}

/// A lost session's sign in screen and the next session start clean: no card, menu or cached pictures carry over.
#[gpui::test]
fn a_lost_session_resets_the_view(cx: &mut TestAppContext) {
    let (view, cx, _rx) = open(cx);
    signed_in(&view, cx);
    click(cx, "person-2-b");
    click(cx, "account");
    focus_composer(&view, cx);
    cx.simulate_input("draft");
    apply(&view, cx, Event::SessionEnded);
    view.read_with(cx, |v, cx| {
        assert!(v.person.is_none());
        assert!(!v.side.account_open);
        assert!(v.images.is_empty() && v.drafts.is_empty());
        assert_eq!(v.composer.read(cx).value(), "");
    });
}

/// The device lists drop ALSA's plugins and the same card listed under several ids, and keep the order.
#[test]
fn device_lists_keep_only_real_devices_once() {
    let pairs = [
        (
            "null",
            "Discard all samples (playback) or generate zero samples (capture)",
        ),
        (
            "lavrate",
            "Rate Converter Plugin Using Libav/FFmpeg Library",
        ),
        ("jack", "JACK Audio Connection Kit"),
        ("pipewire", "PipeWire Sound Server"),
        (
            "default",
            "Default ALSA Output (currently PipeWire Media Server)",
        ),
        ("sysdefault:CARD=PCH", "HDA Intel PCH, ALC3234 Analog"),
        ("front:CARD=PCH,DEV=0", "HDA Intel PCH, ALC3234 Analog"),
        ("dmix:CARD=PCH,DEV=0", "HDA Intel PCH, ALC3234 Analog"),
        ("hdmi:CARD=PCH,DEV=0", "HDA Intel PCH, HDMI 0"),
        ("hw:CARD=PCH,DEV=0", "HDA Intel PCH, ALC3234 Analog"),
        ("{0.0.1.00000000}.{abc}", "Speakers (Realtek Audio)"),
    ];
    let names =
        crate::voice::useful_names(pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())));
    assert_eq!(
        names,
        [
            "PipeWire Sound Server",
            "HDA Intel PCH, ALC3234 Analog",
            "HDA Intel PCH, HDMI 0",
            "Speakers (Realtek Audio)"
        ]
    );
}

/// Admin is in the account menu for staff only; a moderator gets Invites and Members, the owner every section, and
/// a moderator sent to an owner page is told so.
#[gpui::test]
fn admin_opens_by_role(cx: &mut TestAppContext) {
    use super::admin::{Section, sections_for};
    use super::settings::At;
    use vesper_protocol::Role;

    assert!(sections_for(Role::Member).is_empty());
    assert_eq!(
        sections_for(Role::Moderator),
        [Section::Invites, Section::Members]
    );
    assert_eq!(sections_for(Role::Owner).len(), 5);

    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    click(cx, "account");
    assert!(
        cx.debug_bounds("menu-admin").is_none(),
        "a member has no Admin"
    );

    view.update(cx, |v, cx| {
        v.side.account_open = false;
        let id = v.model.state.my_id().to_string();
        v.model.state.users.get_mut(&id).unwrap().role = Role::Moderator;
        cx.notify();
    });
    cx.run_until_parked();
    drain(&mut rx);
    click(cx, "account");
    click(cx, "menu-admin");
    assert!(cx.debug_bounds("nav-invites").is_some() && cx.debug_bounds("nav-members").is_some());
    assert!(
        cx.debug_bounds("nav-rooms").is_none(),
        "Rooms is the owner's"
    );
    let paths: Vec<String> = drain(&mut rx)
        .into_iter()
        .filter_map(|c| match c {
            Cmd::Http { path, .. } => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(paths, ["/api/invites", "/api/users"]);

    view.update_in(cx, |v, window, cx| {
        v.open_page(At::Admin(Section::Server), window, cx)
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("admin-denied").is_some());
    assert!(
        drain(&mut rx).is_empty(),
        "nothing asked for a page it cannot open"
    );
}

/// An invite link comes back once: shown with Copy link, never kept past Done.
#[gpui::test]
fn a_new_invite_link_shows_once(cx: &mut TestAppContext) {
    use super::admin::Section;
    use super::settings::At;
    use vesper_protocol::Role;

    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    view.update_in(cx, |v, window, cx| {
        let id = v.model.state.my_id().to_string();
        v.model.state.users.get_mut(&id).unwrap().role = Role::Owner;
        v.open_page(At::Admin(Section::Invites), window, cx);
    });
    cx.run_until_parked();
    apply(
        &view,
        cx,
        Event::Http {
            tag: "admin:invites".into(),
            result: Ok(crate::net::Json(json!([]))),
        },
    );
    drain(&mut rx);
    click(cx, "invite-create");
    let sent = drain(&mut rx);
    assert!(
        matches!(sent.as_slice(), [Cmd::Http { path, body: Some(b), .. }]
            if path == "/api/invites" && b.0 == json!({ "role": "member", "max_uses": 1, "expires_hours": 168 })),
        "{sent:?}"
    );
    let info = json!({ "id": 9, "role": "member", "uses": 0, "max_uses": 1, "expires_at": 4_000_000_000_000_i64,
                       "created_by": "a", "revoked": false });
    let created = json!({ "info": info, "url": "https://v.example/invite#test-code" });
    apply(
        &view,
        cx,
        Event::Http {
            tag: "admin:create".into(),
            result: Ok(crate::net::Json(created)),
        },
    );
    assert!(cx.debug_bounds("admin-link").is_some());
    let shown = format!("{sent:?}");
    assert!(!shown.contains("test-code"));
    click(cx, "link-done");
    view.read_with(cx, |v, _| assert!(v.page.admin_link_gone()));
}

/// Settings > Account starts from your profile and saves only what changed.
#[gpui::test]
fn the_account_form_starts_from_the_profile(cx: &mut TestAppContext) {
    use super::settings::{At, Section};
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    view.update_in(cx, |v, window, cx| {
        v.open_page(At::Settings(Section::Account), window, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert_eq!(v.input_text("account-name", cx), "Ann")
    });
    drain(&mut rx);
    click(cx, "swatch-3");
    click(cx, "account-save");
    let sent = drain(&mut rx);
    assert!(
        matches!(sent.as_slice(), [Cmd::Http { path, body: Some(b), .. }]
            if path == "/api/me" && b.0 == json!({ "accent": "#99b860" })),
        "{sent:?}"
    );
}

/// Profile pictures: fetched once per version, a late answer for an old version is dropped, a new version shows the
/// letters until it comes, and a removed picture is let go.
#[gpui::test]
fn profile_pictures_follow_their_version(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    drain(&mut rx);
    let set = |v: Option<&str>| {
        let mut u = user("b", "Bob");
        u["avatar"] = json!(v);
        Event::Server(Box::new(
            serde_json::from_value(json!({ "type": "user_updated", "user": u })).unwrap(),
        ))
    };
    let fetches = |rx: &mut UnboundedReceiver<Cmd>| -> Vec<String> {
        drain(rx)
            .into_iter()
            .filter_map(|c| match c {
                Cmd::FetchAvatar { user_id, version } => Some(format!("{user_id}@{version}")),
                _ => None,
            })
            .collect()
    };
    let shown = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| cx.global::<super::c::Pictures>().ready("b").is_some())
    };
    let came = |version: &str| Event::Avatar {
        user_id: "b".into(),
        version: version.into(),
        picture: Some(crate::files::Picture {
            width: 1,
            height: 1,
            bgra: vec![255; 4],
        }),
    };
    apply(&view, cx, set(Some("v1")));
    assert_eq!(fetches(&mut rx), ["b@v1"]);
    apply(&view, cx, set(Some("v1")));
    assert!(
        fetches(&mut rx).is_empty(),
        "the same version is not fetched again"
    );
    apply(&view, cx, came("v1"));
    assert!(shown(cx));
    apply(&view, cx, set(Some("v2")));
    assert_eq!(fetches(&mut rx), ["b@v2"]);
    assert!(!shown(cx), "the letters show until the new picture comes");
    apply(&view, cx, came("v1"));
    assert!(!shown(cx), "a late answer for the old picture is dropped");
    apply(&view, cx, came("v2"));
    assert!(shown(cx));
    apply(&view, cx, set(None));
    assert!(!shown(cx));
    assert!(fetches(&mut rx).is_empty());
}

/// Ctrl+K opens the switcher over a focused composer, the typed text ranks the rows, Enter opens the room; the `r `
/// chip scopes it and Backspace removes the chip; Esc closes.
#[gpui::test]
fn the_switcher_finds_and_opens_a_room(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    focus_composer(&view, cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-k"
    } else {
        "ctrl-k"
    });
    assert!(cx.debug_bounds("switcher").is_some());
    cx.simulate_input("hang");
    view.read_with(cx, |v, cx| {
        let r = v.switcher_results(cx);
        assert_eq!(r[0].label, "hangout", "{r:?}");
    });
    drain(&mut rx);
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |v, _| {
        assert_eq!(v.find.layer, None);
        assert_eq!(v.model.active, Some(2));
    });

    view.update_in(cx, |v, w, cx| {
        v.open_layer(super::find::Layer::Switcher, w, cx)
    });
    cx.simulate_input("p ");
    view.read_with(cx, |v, cx| {
        assert_eq!(v.find.scope_kind(), Some(crate::switcher::Kind::Person));
        let r = v.switcher_results(cx);
        assert!(r.iter().all(|i| i.kind == crate::switcher::Kind::Person) && !r.is_empty());
    });
    cx.simulate_keystrokes("backspace");
    view.read_with(cx, |v, _| assert_eq!(v.find.scope_kind(), None));
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert_eq!(v.find.layer, None));
}

/// Search waits 300 ms after typing, drops a late answer, shows the hits, and Enter jumps to the first: its room
/// opens and older pages load until the message is there.
#[gpui::test]
fn search_asks_after_a_pause_and_jumps(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    signed_in(&view, cx);
    view.update_in(cx, |v, w, cx| {
        v.open_layer(super::find::Layer::Search, w, cx)
    });
    assert!(cx.debug_bounds("search").is_some());
    drain(&mut rx);
    cx.simulate_input("cable");
    assert!(
        !drain(&mut rx)
            .iter()
            .any(|c| matches!(c, Cmd::Search { .. })),
        "not before the pause"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(350));
    cx.run_until_parked();
    let seq = drain(&mut rx)
        .into_iter()
        .find_map(|c| match c {
            Cmd::Search { seq, q } => {
                assert_eq!(q.q, "cable");
                Some(seq)
            }
            _ => None,
        })
        .expect("a search");
    let page = |id: i64| -> vesper_protocol::SearchPage {
        let mut m = msg(id, "b", "the good cable");
        m["room_id"] = json!(2);
        serde_json::from_value(
            json!({ "hits": [{ "message": m, "snippet": "the good \u{2}cable\u{3}" }],
            "next_before": null }),
        )
        .unwrap()
    };
    apply(
        &view,
        cx,
        Event::Searched {
            seq: seq - 1,
            page: Ok(page(99)),
        },
    );
    view.read_with(cx, |v, _| {
        assert!(v.find.search.hits.is_empty(), "an older answer is dropped")
    });
    apply(
        &view,
        cx,
        Event::Searched {
            seq,
            page: Ok(page(5)),
        },
    );
    assert!(cx.debug_bounds("search-hit-0").is_some());
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |v, _| assert_eq!(v.find.layer, None));
    let cmds = drain(&mut rx);
    assert!(
        cmds.iter().any(|c| matches!(c, Cmd::LoadLatest(2))),
        "{cmds:?}"
    );
    let older: MessagePage = serde_json::from_value(json!({ "messages": [msg(20, "b", "x")],
        "more_before": true, "more_after": false }))
    .unwrap();
    apply(
        &view,
        cx,
        Event::Latest {
            room: 2,
            page: older,
        },
    );
    assert!(drain(&mut rx).iter().any(|c| matches!(
        c,
        Cmd::LoadOlder {
            room: 2,
            before: 20
        }
    )));
    let mut found = msg(5, "b", "the good cable");
    found["room_id"] = json!(2);
    let page: MessagePage =
        serde_json::from_value(json!({ "messages": [found], "more_before": false,
        "more_after": false }))
        .unwrap();
    view.update_in(cx, |v, window, cx| {
        v.apply(Event::Older { room: 2, page }, window, cx);
        assert_eq!(v.model.jump, Some(5));
    });
}

/// A click on a picture opens the viewer in the app (not the system's), which fetches the full size once; Esc closes
/// it and lets the picture go.
#[gpui::test]
fn a_picture_opens_in_the_viewer(cx: &mut TestAppContext) {
    let (view, cx, mut rx) = open(cx);
    chat_room(&view, cx);
    apply(
        &view,
        cx,
        Event::Image {
            id: IMAGE_ID.into(),
            picture: Some(crate::files::Picture {
                width: 2,
                height: 2,
                bgra: vec![255; 16],
            }),
        },
    );
    drain(&mut rx);
    let b = cx
        .debug_bounds(tagged(format!("image-{IMAGE_ID}")))
        .unwrap();
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
    let cmds = drain(&mut rx);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::FetchFull(id) if id == IMAGE_ID)),
        "{cmds:?}"
    );
    assert!(
        !cmds.iter().any(|c| matches!(c, Cmd::Download { .. })),
        "not the system's viewer"
    );
    apply(
        &view,
        cx,
        Event::Full {
            id: IMAGE_ID.into(),
            picture: Some(crate::files::Picture {
                width: 4,
                height: 2,
                bgra: vec![255; 32],
            }),
        },
    );
    assert!(cx.debug_bounds("viewer").is_some());
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert!(v.viewer.is_none()));
}

/// The switcher's Settings rows open their section; Admin rows are there for staff only, a moderator's without the
/// owner's sections, and open theirs.
#[gpui::test]
fn the_switcher_opens_settings_and_admin_sections(cx: &mut TestAppContext) {
    use super::admin::Section as Admin;
    use super::find::Layer;
    use super::settings::{At, Section};
    use vesper_protocol::Role;

    let (view, cx, _rx) = open(cx);
    signed_in(&view, cx);
    let labels = |view: &Entity<Vesper>, cx: &mut VisualTestContext, q: &str| {
        view.update_in(cx, |v, w, cx| v.open_layer(Layer::Switcher, w, cx));
        cx.simulate_input(q);
        let r: Vec<String> = view.read_with(cx, |v, cx| {
            v.switcher_results(cx)
                .into_iter()
                .map(|i| i.label)
                .collect()
        });
        r
    };
    let as_role = |view: &Entity<Vesper>, cx: &mut VisualTestContext, role: Role| {
        view.update(cx, |v, cx| {
            let id = v.model.state.my_id().to_string();
            v.model.state.users.get_mut(&id).unwrap().role = role;
            cx.notify();
        });
    };

    assert_eq!(
        labels(&view, cx, "settings: appea")[0],
        "Settings: Appearance"
    );
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |v, _| {
        assert!(v.settings_open);
        assert_eq!(v.page.at, At::Settings(Section::Appearance));
    });
    assert!(
        !labels(&view, cx, "> ")
            .iter()
            .any(|l| l.starts_with("Admin: ")),
        "a member has no Admin rows"
    );
    cx.simulate_keystrokes("escape");

    as_role(&view, cx, Role::Moderator);
    let admin: Vec<String> = labels(&view, cx, "> admin")
        .into_iter()
        .filter(|l| l.starts_with("Admin: "))
        .collect();
    assert_eq!(admin, ["Admin: Invites", "Admin: Members"]);
    cx.simulate_keystrokes("escape");

    as_role(&view, cx, Role::Owner);
    assert_eq!(labels(&view, cx, "admin: disk")[0], "Admin: Disk use");
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |v, _| assert_eq!(v.page.at, At::Admin(Admin::Disk)));
}
