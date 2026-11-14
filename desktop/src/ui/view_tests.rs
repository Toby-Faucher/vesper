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
        matches!(cmds.as_slice(), [Cmd::Send { room: 1, body }] if body == "hello\nworld"),
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
    view.read_with(cx, |v, _| assert_eq!(v.model.active, Some(2)));
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
        "leave-voice",
        "call-person-a",
        "call-person-b",
        "volume-b",
        "speaking-b",
    ] {
        assert!(cx.debug_bounds(s).is_some(), "{s} is drawn");
    }
    assert!(
        cx.debug_bounds("volume-a").is_none(),
        "no slider for myself"
    );
    assert!(cx.debug_bounds("speaking-a").is_none());
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
        "settings-mic-0",
        "settings-speakers",
    ] {
        assert!(cx.debug_bounds(s).is_some(), "{s} is drawn");
    }
    assert!(
        cx.debug_bounds("settings-ptt-key").is_none(),
        "voice activity: no key"
    );

    click(cx, "settings-gate");
    click(cx, "settings-echo-off");
    click(cx, "settings-mic-0");
    click(cx, "settings-speakers");
    view.read_with(cx, |v, _| {
        let s = &v.model.settings;
        assert_eq!(s.denoise, Denoise::Gate);
        assert!(!s.echo_cancel);
        assert_eq!(s.input_device.as_deref(), Some("USB mic"));
        assert_eq!(s.output_device, None, "back to the default");
    });

    click(cx, "settings-ptt");
    assert_eq!(
        watched.try_iter().last(),
        Some(KeyMsg::Watch(Some(device_query::Keycode::Grave)))
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
    assert_eq!(
        watched.try_iter().last(),
        Some(KeyMsg::Watch(Some(device_query::Keycode::V)))
    );

    // Out of the settings, the window's own V presses and releases push to talk (Wayland has only these).
    click(cx, "settings-close");
    cx.simulate_keystrokes("v");
    // GPUI's test keystrokes are key downs only.
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("v").unwrap(),
    });
    let got: Vec<KeyMsg> = watched.try_iter().collect();
    assert_eq!(got, [KeyMsg::Window(true), KeyMsg::Window(false)]);
    cx.simulate_keystrokes("b");
    assert!(watched.try_iter().next().is_none(), "another key");

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
    view.update(cx, |v, cx| v.act(|m| m.set_ptt(true), cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("call-ptt").is_some());
    assert!(matches!(
        drain(&mut rx).as_slice(),
        [Cmd::Voice(VoiceCmd::Muted(false))]
    ));
}
