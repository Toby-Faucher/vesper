//! Calls between desktop clients through a real `vesper-server` and its SFU on loopback: str0m, Opus, the jitter
//! buffer and the mix, with a sine for a microphone and a recorder for speakers.
#![cfg(unix)] // the server crates use Unix file permissions; Windows CI runs the unit tests

mod common;

use std::sync::Arc;

use common::{Client, two, until_all};
use vesper_desktop::model::{CALL_ENDED, Phase};
use vesper_desktop::voice::RATE;

/// Level and pitch of what was played. The pitch counts zero crossings, enough for one clean tone.
fn analyse(x: &[f32]) -> (f32, f32) {
    let rms = (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt();
    let crossings = x
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    let hz = crossings as f32 * RATE as f32 / 2.0 / x.len().max(1) as f32;
    (rms, hz)
}

/// The last 200 ms played are a clear tone near `hz`.
fn hears(c: &Client, hz: f32) -> bool {
    let (rms, got) = analyse(&c.audio.last(200));
    rms > 0.1 && (got - hz).abs() < 20.0
}

/// The last 200 ms played are silence.
fn silent(c: &Client) -> bool {
    let x = c.audio.last(200);
    x.len() == RATE as usize / 5 && analyse(&x).0 < 0.005
}

fn join(c: &mut Client, room: i64) {
    let cmds = c.model.join_voice(room);
    c.run(cmds);
}

fn peer_muted(c: &Client, user: &str) -> bool {
    c.model
        .state
        .voice
        .iter()
        .any(|p| p.user_id == user && p.muted)
}

#[tokio::test(flavor = "multi_thread")]
async fn two_engines_hear_each_other_mute_and_leave() {
    let (_s, mut a, mut b) = two().await;
    let room = a.room("general");
    let (ann, bob) = (a.id(), b.id());
    a.audio.set_tone(Some(440.0));
    join(&mut a, room);
    join(&mut b, room);
    assert!(
        !a.model.call.headphones,
        "echo cancellation is on, so no headphones note"
    );
    until_all(&mut [&mut a, &mut b], "both connected", |c| {
        c.iter()
            .all(|c| c.model.call.phase == Phase::Joined && c.model.call.connected)
    })
    .await;

    // A to B: a 440 Hz tone, and B sees A speaking under A's user id.
    until_all(&mut [&mut a, &mut b], "bob hears ann", |c| {
        hears(c[1], 440.0) && c[1].model.call.speaking == [ann.clone()]
    })
    .await;
    let (rms, hz) = analyse(&b.audio.last(200));
    println!("bob hears {rms:.3} rms at {hz:.0} Hz");
    // The engine's 2 s stats: the call's round trip, and a grade for the other person once two windows came.
    until_all(&mut [&mut a, &mut b], "stats", |c| {
        c.iter().all(|c| {
            c.model.call.quality.is_some_and(|q| q.rtt_ms.is_some())
                && c.model.call.links.values().any(|l| l.grade.is_some())
        })
    })
    .await;
    assert!(a.model.call.me_speaking && !b.model.call.me_speaking);

    // B to A: a 660 Hz tone; ann goes quiet at bob.
    a.audio.set_tone(None);
    b.audio.set_tone(Some(660.0));
    until_all(&mut [&mut a, &mut b], "ann hears bob", |c| {
        hears(c[0], 660.0) && c[0].model.call.speaking == [bob.clone()] && silent(c[1])
    })
    .await;
    let (rms, hz) = analyse(&a.audio.last(200));
    println!("ann hears {rms:.3} rms at {hz:.0} Hz");

    // Mute stops ann's audio at bob, and bob sees her muted.
    b.audio.set_tone(None);
    a.audio.set_tone(Some(440.0));
    until_all(&mut [&mut a, &mut b], "bob hears ann again", |c| {
        hears(c[1], 440.0)
    })
    .await;
    let cmds = a.model.toggle_mute();
    a.run(cmds);
    until_all(&mut [&mut a, &mut b], "ann muted at bob", |c| {
        silent(c[1]) && peer_muted(c[1], &ann) && c[1].model.call.speaking.is_empty()
    })
    .await;
    assert!(!a.model.call.me_speaking, "muted is not speaking");

    // Deafen silences what bob plays, though ann is heard again.
    let cmds = a.model.toggle_mute();
    a.run(cmds);
    let cmds = b.model.toggle_deafen();
    b.run(cmds);
    until_all(&mut [&mut a, &mut b], "bob deafened", |c| {
        c[1].model.call.speaking == [ann.clone()] && silent(c[1])
    })
    .await;

    // Leave: bob sees ann go, and her engine stops.
    let cmds = a.model.leave_voice();
    a.run(cmds);
    assert_eq!(a.model.call.phase, Phase::Idle);
    until_all(&mut [&mut a, &mut b], "ann left", |c| {
        !c[1].model.state.voice.iter().any(|p| p.user_id == ann)
    })
    .await;
    // The engine sees the stop within one 20 ms turn.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let played = a.audio.played.lock().unwrap().len();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        a.audio.played.lock().unwrap().len(),
        played,
        "nothing plays after leave"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn each_stream_maps_to_its_speaker_with_its_own_volume() {
    let (s, mut a, mut b) = two().await;
    s.signup("carol", false).await;
    let mut c = Client::start(Arc::default()).await;
    c.sign_in(&s.origin, "carol").await;
    let room = a.room("general");
    let (ann, cy) = (a.id(), c.id());
    for x in [&mut a, &mut b, &mut c] {
        join(x, room);
    }
    until_all(&mut [&mut a, &mut b, &mut c], "all connected", |c| {
        c.iter().all(|c| c.model.call.connected)
    })
    .await;

    a.audio.set_tone(Some(440.0));
    until_all(&mut [&mut a, &mut b, &mut c], "bob hears ann only", |c| {
        hears(c[1], 440.0) && c[1].model.call.speaking == [ann.clone()]
    })
    .await;
    a.audio.set_tone(None);
    c.audio.set_tone(Some(660.0));
    until_all(&mut [&mut a, &mut b, &mut c], "bob hears cy only", |c| {
        hears(c[1], 660.0) && c[1].model.call.speaking == [cy.clone()]
    })
    .await;

    // Cy at 0 % for bob: still speaking, not heard. Ann still hears cy.
    let cmds = b.model.set_volume(&cy, 0.0);
    b.run(cmds);
    until_all(&mut [&mut a, &mut b, &mut c], "cy silent for bob", |c| {
        silent(c[1]) && c[1].model.call.speaking == [cy.clone()] && hears(c[0], 660.0)
    })
    .await;
    // And loud is limited: 400 % stays under the ceiling.
    let cmds = b.model.set_volume(&cy, 400.0);
    b.run(cmds);
    until_all(&mut [&mut a, &mut b, &mut c], "cy loud for bob", |c| {
        hears(c[1], 660.0)
    })
    .await;
    let peak = b.audio.last(200).iter().fold(0f32, |m, x| m.max(x.abs()));
    assert!(peak <= 0.71, "limited: {peak}");
}

#[tokio::test(flavor = "multi_thread")]
async fn force_mute_a_lost_socket_and_another_device() {
    let (s, mut a, mut b) = two().await;
    let room = a.room("general");
    let (ann, bob) = (a.id(), b.id());
    join(&mut a, room);
    join(&mut b, room);
    b.audio.set_tone(Some(440.0));
    until_all(&mut [&mut a, &mut b], "ann hears bob", |c| {
        hears(c[0], 440.0)
    })
    .await;

    // A moderator's force mute: bob is muted and cannot unmute.
    assert!(s.state.voice.force_mute(&bob, true));
    until_all(&mut [&mut a, &mut b], "bob force muted", |c| {
        c[1].model.call.force_muted && c[1].model.call.muted && silent(c[0])
    })
    .await;
    assert!(b.model.toggle_mute().is_empty());
    s.state.voice.force_mute(&bob, false);
    until_all(&mut [&mut a, &mut b], "lifted", |c| {
        !c[1].model.call.force_muted
    })
    .await;
    assert!(b.model.call.muted, "lifting does not unmute");
    let cmds = b.model.toggle_mute();
    b.run(cmds);
    until_all(&mut [&mut a, &mut b], "bob heard again", |c| {
        hears(c[0], 440.0)
    })
    .await;

    // The server drops bob's socket: the call goes with it and comes back with the next socket.
    s.state.hub.close_user(&bob, 1012);
    until_all(&mut [&mut a, &mut b], "bob waits to rejoin", |c| {
        c[1].model.call.rejoin.is_some()
    })
    .await;
    until_all(&mut [&mut a, &mut b], "bob rejoined and heard", |c| {
        c[1].model.call.connected && hears(c[0], 440.0)
    })
    .await;

    // Ann joins from another device: this one's call ends.
    let mut other = Client::start(Arc::default()).await;
    other.sign_in(&s.origin, "ann").await;
    join(&mut other, room);
    until_all(
        &mut [&mut a, &mut b, &mut other],
        "ann's first call ends",
        |c| c[0].model.call.phase == Phase::Idle && c[2].model.call.connected,
    )
    .await;
    assert_eq!(a.model.call.error, CALL_ENDED);
    // The old session's leave and the new one's join reach bob as two messages: wait for the join.
    until_all(
        &mut [&mut a, &mut b, &mut other],
        "bob sees ann in the call again",
        |c| c[1].model.state.voice.iter().any(|p| p.user_id == ann),
    )
    .await;
}

/// DeepFilterNet chosen, but this test server has no `/dsp/model.tar.gz`: the call goes on with the noise gate, says
/// why, and the tone still reaches the other side.
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_model_falls_back_to_the_gate_and_says_so() {
    use vesper_desktop::settings::Denoise;
    use vesper_desktop::voice::denoise::DEEPFILTER;

    let (_s, mut a, mut b) = two().await;
    let room = a.room("general");
    a.model.settings.denoise = Denoise::DeepFilter;
    a.audio.set_tone(Some(440.0));
    join(&mut a, room);
    join(&mut b, room);
    until_all(&mut [&mut a, &mut b], "ann's filter settles", |c| {
        c[0].model.call.filter != vesper_desktop::voice::denoise::LOADING
            && !c[0].model.call.filter.is_empty()
    })
    .await;
    let filter = a.model.call.filter.clone();
    println!("ann's filter: {filter}");
    if filter == DEEPFILTER {
        // A model cached in this machine's data dir from a real run: DeepFilterNet takes a steady tone for noise.
        println!("NOTE: a cached model was found, so the fallback was not exercised");
        return;
    }
    assert!(
        filter.starts_with("DeepFilterNet is not available"),
        "{filter}"
    );
    until_all(
        &mut [&mut a, &mut b],
        "bob hears ann through the gate",
        |c| hears(c[1], 440.0),
    )
    .await;
}
