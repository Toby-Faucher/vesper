//! Chat against a real `vesper-server`: edits, deletes, reactions, replies, mentions, typing and files, each seen live
//! by a second client. No window: the clients drive `Model` and the network thread, as the views do.
#![cfg(unix)] // the server crates use Unix file permissions; Windows CI runs the unit tests

mod common;

use std::time::{Duration, Instant};

use common::{Client, two};
use vesper_desktop::files::Picture;
use vesper_desktop::model::EMOJIS;
use vesper_desktop::net::{Cmd, Event};
use vesper_protocol::{AttachmentKind, Message};

fn find<'a>(c: &'a Client, room: i64, body: &str) -> Option<&'a Message> {
    c.model
        .chat
        .rooms
        .get(&room)?
        .messages
        .iter()
        .find(|m| m.body == body)
}

fn by_id(c: &Client, room: i64, id: i64) -> Message {
    c.model.chat.rooms[&room]
        .messages
        .iter()
        .find(|m| m.id == id)
        .cloned()
        .unwrap()
}

async fn send(c: &mut Client, body: &str) {
    let cmds = c.model.send(body);
    c.run(cmds);
}

/// Pumps `c` until `pick` takes an event the model does not keep (a picture, a download).
async fn event<T>(c: &mut Client, what: &str, mut pick: impl FnMut(&Event) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut got = None;
        c.pump_with(|ev| {
            if got.is_none() {
                got = pick(ev);
            }
            vec![]
        });
        if let Some(v) = got {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_delete_and_reactions_are_seen_live() {
    let (_s, mut a, mut b) = two().await;
    let g = a.room("general");
    send(&mut a, "first").await;
    b.until("the message", |m| {
        m.chat
            .rooms
            .get(&g)
            .is_some_and(|l| l.messages.iter().any(|x| x.body == "first"))
    })
    .await;
    let id = find(&b, g, "first").unwrap().id;

    // Ann edits; Bob sees the new text marked edited.
    a.until("her own copy", |m| {
        m.chat.rooms[&g].messages.iter().any(|x| x.id == id)
    })
    .await;
    assert_eq!(a.model.edit_last().map(|e| e.0), Some(id));
    let cmds = a.model.save_edit("first, edited");
    a.run(cmds);
    a.until("the edit closes", |m| m.editing.is_none()).await;
    b.until("the edit", |m| {
        m.chat.rooms[&g]
            .messages
            .iter()
            .any(|x| x.body == "first, edited")
    })
    .await;
    assert!(by_id(&b, g, id).edited_at.is_some());

    // Bob reacts; Ann sees who. Bob takes it off again.
    let cmds = b.model.react(id, EMOJIS[0]);
    b.run(cmds);
    let bob = b.id();
    a.until("the reaction", |m| {
        m.chat.rooms[&g].messages.iter().any(|x| {
            x.id == id
                && x.reactions
                    .iter()
                    .any(|r| r.emoji == EMOJIS[0] && r.user_ids == [bob.clone()])
        })
    })
    .await;
    b.until("his own copy", |m| {
        m.chat.rooms[&g]
            .messages
            .iter()
            .any(|x| x.id == id && !x.reactions.is_empty())
    })
    .await;
    let cmds = b.model.react(id, EMOJIS[0]);
    b.run(cmds);
    a.until("the reaction goes", |m| {
        m.chat.rooms[&g]
            .messages
            .iter()
            .any(|x| x.id == id && x.reactions.is_empty())
    })
    .await;

    // Bob is a member: the app offers no delete of Ann's message, and the server refuses one sent anyway.
    assert!(b.model.delete(id, true).is_empty());
    b.run(vec![Cmd::Delete(id)]);
    b.until("the refusal", |m| !m.error.is_empty()).await;
    assert_eq!(b.model.error, "You are not allowed to do that.");

    // Ann deletes her own, after the confirm.
    assert!(a.model.delete(id, false).is_empty());
    let cmds = a.model.delete(id, true);
    a.run(cmds);
    b.until("the delete", |m| {
        m.chat.rooms[&g]
            .messages
            .iter()
            .any(|x| x.id == id && x.deleted)
    })
    .await;
    assert_eq!(by_id(&b, g, id).body, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_round_trip_and_a_mention_counted() {
    let (_s, mut a, mut b) = two().await;
    let g = a.room("general");
    send(&mut a, "question **bold**").await;
    b.until("the question", |m| {
        m.chat.rooms.get(&g).is_some_and(|l| !l.messages.is_empty())
    })
    .await;
    let q = find(&b, g, "question **bold**").unwrap().id;
    b.model.reply(q);
    send(&mut b, "answer").await;
    b.until("the reply bar closes", |m| m.reply_to.is_none())
        .await;
    a.until("the reply", |m| {
        m.chat.rooms[&g].messages.iter().any(|x| x.body == "answer")
    })
    .await;
    let r = find(&a, g, "answer").unwrap().reply_to.clone().unwrap();
    assert_eq!(
        (r.id, r.author_id, r.excerpt.as_str()),
        (q, a.id(), "question **bold**")
    );
    assert!(
        a.model.jump_to(r.id),
        "the quoted message is loaded: a click jumps to it"
    );

    // A mention of Ann in a room she is not looking at counts as an unread mention.
    let cmds = a.model.open_room(a.room("hangout"));
    a.run(cmds);
    send(&mut b, "@ann look").await;
    a.until("the mention", |m| {
        m.state.unread.get(&g).is_some_and(|u| u.mentions == 1)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_is_seen_by_the_other_client() {
    let (_s, mut a, mut b) = two().await;
    let g = a.room("general");
    let cmds = a.model.typed("h", Instant::now());
    assert_eq!(cmds.len(), 1);
    a.run(cmds);
    assert!(a.model.typed("he", Instant::now()).is_empty(), "throttled");
    b.until("ann is typing", |m| m.typers(g, Instant::now()) == ["ann"])
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn files_upload_attach_and_download_with_the_cookie() {
    let (s, mut a, mut b) = two().await;
    let g = a.room("general");
    let dir = tempfile::tempdir().unwrap();

    // A small text file, sent with no text: the body is its name, the kind is file.
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "hello file").unwrap();
    let cmds = a.model.attach(vec![(notes.clone(), 10)]);
    a.run(cmds);
    a.until("uploaded", |m| !m.tray.done(g).is_empty()).await;
    send(&mut a, "").await;
    a.until("the tray empties", |m| m.tray.items.is_empty())
        .await;
    b.until("the file", |m| {
        m.chat.rooms[&g]
            .messages
            .iter()
            .any(|x| !x.attachments.is_empty())
    })
    .await;
    let f = find(&b, g, "notes.txt").unwrap().attachments[0].clone();
    assert_eq!(
        (f.name.as_str(), f.kind, f.size),
        ("notes.txt", AttachmentKind::File, 10)
    );

    // Bob saves it.
    let to = dir.path().join("saved").join("notes.txt");
    b.run(vec![Cmd::Download {
        id: f.id.clone(),
        to: to.clone(),
        open: false,
    }]);
    b.until("saved", |m| m.saved.is_some()).await;
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "hello file");

    // An image: the server sees a png, Bob's client fetches it with his cookie and decodes it small.
    let pic = dir.path().join("pic.png");
    image::RgbaImage::from_pixel(800, 400, image::Rgba([0, 128, 255, 255]))
        .save(&pic)
        .unwrap();
    let size = std::fs::metadata(&pic).unwrap().len();
    let cmds = a.model.attach(vec![(pic, size)]);
    a.run(cmds);
    a.until("uploaded", |m| !m.tray.done(g).is_empty()).await;
    send(&mut a, "a picture").await;
    b.until("the image", |m| {
        m.chat.rooms[&g]
            .messages
            .iter()
            .any(|x| x.body == "a picture")
    })
    .await;
    let img = find(&b, g, "a picture").unwrap().attachments[0].clone();
    assert_eq!(
        (img.kind, img.mime.as_str()),
        (AttachmentKind::Image, "image/png")
    );
    b.run(vec![Cmd::FetchImage(img.id.clone())]);
    let p: Picture = event(&mut b, "the picture", |ev| match ev {
        Event::Image { id, picture } if *id == img.id => Some(picture.clone().expect("it decodes")),
        _ => None,
    })
    .await;
    assert_eq!((p.width, p.height), (720, 360));
    assert_eq!(&p.bgra[..4], [255, 128, 0, 255], "BGRA");

    // Without the cookie the same route is refused.
    let res = s
        .http()
        .get(format!("{}/api/uploads/{}", s.origin, img.id))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_upload_leaves_the_tray_and_too_large_is_refused_before_sending() {
    let (_s, mut a, _b) = two().await;
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.bin");
    std::fs::write(&big, vec![0u8; 4 << 20]).unwrap();
    let cmds = a.model.attach(vec![(big.clone(), 4 << 20)]);
    a.run(cmds);
    let key = a.model.tray.items[0].key;
    let cmds = a.model.remove_upload(key);
    assert!(matches!(cmds.as_slice(), [Cmd::CancelUpload(k)] if *k == key));
    a.run(cmds);
    assert!(a.model.tray.items.is_empty());

    a.model.state.me.as_mut().unwrap().upload_max = 1 << 20;
    assert!(a.model.attach(vec![(big, 4 << 20)]).is_empty());
    assert_eq!(a.model.tray.items[0].error, "Files can be up to 1 MB.");
}
