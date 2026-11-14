//! The views in GPUI's headless test window, with fake data: no display and no server.

use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use gpui_component::Root;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use vesper_protocol::{Me, MessagePage, ServerMsg};

use super::Vesper;
use crate::model::Screen;
use crate::net::{Cmd, Event};

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
