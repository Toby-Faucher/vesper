//! The SFU loop: owns the UDP socket and every client, routes packets, and forwards media
//! between clients of the same room. Adapted from str0m's `examples/chat.rs` `run`.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use str0m::change::SdpOffer;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Input, Rtc};
use tokio::sync::mpsc as tmpsc;
use tracing::{debug, warn};

use crate::client::{Client, Propagated};
use crate::{Command, JoinError, JoinOk, SfuConfig, SfuEvent};

/// Upper bound on one loop iteration's socket wait, so commands (joins, answers) are picked up
/// within this time even when the socket is idle.
const MAX_WAIT: Duration = Duration::from_millis(10);

pub(crate) fn run(socket: UdpSocket, rx: Receiver<Command>, cfg: SfuConfig) {
    let socket_addr = socket.local_addr().expect("bound socket has an address");
    let mut advertised = cfg.advertised;
    let mut clients: Vec<Client> = vec![];
    let mut to_propagate: VecDeque<Propagated> = VecDeque::new();
    let mut buf = vec![0; 2000];

    loop {
        // Dropping a client drops its incoming tracks, which makes the other clients stop them.
        clients.retain_mut(|c| {
            let alive = c.rtc.is_alive();
            if !alive {
                c.close();
            }
            alive
        });

        loop {
            match rx.try_recv() {
                Ok(cmd) => handle_command(
                    cmd,
                    &mut clients,
                    &mut advertised,
                    socket_addr,
                    cfg.answer_timeout,
                ),
                Err(TryRecvError::Empty) => break,
                // Every SfuHandle is gone: shut down, telling clients their sessions ended.
                Err(TryRecvError::Disconnected) => {
                    clients.iter_mut().for_each(Client::close);
                    return;
                }
            }
        }

        let mut timeout = Instant::now() + MAX_WAIT;
        for client in clients.iter_mut() {
            let t = poll_until_timeout(client, &mut to_propagate, &socket);
            timeout = timeout.min(t);
        }

        if let Some(p) = to_propagate.pop_front() {
            propagate(&p, &mut clients);
            continue;
        }

        // The read timeout must not be 0.
        let wait = timeout
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        socket
            .set_read_timeout(Some(wait))
            .expect("setting socket read timeout");

        match socket.recv_from(&mut buf) {
            Ok((n, source)) => route(&buf[..n], source, socket_addr, &mut clients),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            // e.g. ECONNREFUSED from an ICMP unreachable after a client vanished.
            Err(e) => debug!("udp recv failed: {e}"),
        }

        let now = Instant::now();
        for client in &mut clients {
            client.handle_input(Input::Timeout(now));
        }
    }
}

fn receive(data: &[u8], source: SocketAddr, destination: SocketAddr) -> Option<Input<'_>> {
    Some(Input::Receive(
        Instant::now(),
        Receive {
            proto: Protocol::Udp,
            source,
            destination,
            contents: data.try_into().ok()?,
        },
    ))
}

/// Warn about advertised sets clients cannot use to reach a socket bound to `bind`.
pub(crate) fn check_advertised(bind: SocketAddr, advertised: &[SocketAddr]) {
    // A [::] socket is dual-stack (on Linux by default), so it serves both families.
    let dual_stack = bind.is_ipv6() && bind.ip().is_unspecified();
    if advertised.is_empty() {
        warn!("no advertised addresses: clients cannot reach the SFU on {bind}");
    } else if !dual_stack && !advertised.iter().any(|a| a.is_ipv4() == bind.is_ipv4()) {
        warn!("no advertised address {advertised:?} has the same IP family as bind {bind}");
    }
}

fn route(data: &[u8], source: SocketAddr, socket_addr: SocketAddr, clients: &mut [Client]) {
    // A dual-stack socket reports IPv4 peers as ::ffff:a.b.c.d; str0m and the SDP know them as
    // plain IPv4, so canonicalize (Client maps them back when sending).
    let source = SocketAddr::new(source.ip().to_canonical(), source.port());
    // rtc.accepts() demultiplexes on source address / STUN credentials, not destination.
    let Some(probe) = receive(data, source, socket_addr) else {
        return;
    };
    let Some(client) = clients.iter_mut().find(|c| c.rtc.accepts(&probe)) else {
        // Common: a client's first STUN can beat its join being processed.
        debug!("no client accepts udp input from {source}");
        return;
    };
    let destination = client.local_addr_for(source, socket_addr);
    if let Some(input) = receive(data, source, destination) {
        client.handle_input(input);
    }
}

fn handle_command(
    cmd: Command,
    clients: &mut Vec<Client>,
    advertised: &mut Vec<SocketAddr>,
    socket_addr: SocketAddr,
    answer_timeout: Duration,
) {
    match cmd {
        Command::Join {
            room,
            user,
            offer,
            reply,
        } => {
            let (mut client, answer_sdp, events) = match new_client(
                &room,
                &user,
                &offer,
                advertised,
                socket_addr,
                answer_timeout,
            ) {
                Ok(v) => v,
                Err(e) => {
                    let _ = reply.send(Err(e));
                    return;
                }
            };
            if let Some(old) = clients.iter_mut().find(|c| c.user == user) {
                // Superseded: the retain at the top of the loop closes and removes it.
                old.rtc.disconnect();
            }
            // Subscribe the newcomer to everyone already in the room.
            for track in clients
                .iter()
                .filter(|c| c.room == room && c.user != user && c.rtc.is_alive())
                .flat_map(|c| c.tracks_in.iter())
            {
                client.handle_track_open(Arc::downgrade(track));
            }
            if reply.send(Ok(JoinOk { answer_sdp, events })).is_ok() {
                clients.push(client);
            }
        }
        Command::Answer { user, sdp } => {
            match clients
                .iter_mut()
                .find(|c| c.user == user && c.rtc.is_alive())
            {
                Some(c) => c.handle_answer(&sdp),
                None => debug!("answer for unknown user {user}"),
            }
        }
        Command::Leave { user } => {
            // Live only: after a re-join the dead old session may still be in the list.
            if let Some(c) = clients
                .iter_mut()
                .find(|c| c.user == user && c.rtc.is_alive())
            {
                c.rtc.disconnect();
            }
        }
        Command::SetAdvertised(addrs) => {
            check_advertised(socket_addr, &addrs);
            *advertised = addrs;
        }
        #[cfg(feature = "test-support")]
        Command::Panic => panic!("SFU panic requested by a test"),
    }
}

fn new_client(
    room: &str,
    user: &str,
    offer: &str,
    advertised: &[SocketAddr],
    socket_addr: SocketAddr,
    answer_timeout: Duration,
) -> Result<(Client, String, tmpsc::UnboundedReceiver<SfuEvent>), JoinError> {
    let bad = |e: &dyn std::fmt::Display| JoinError::BadOffer(e.to_string());
    // Untrusted input: a voice client needs exactly one audio m-line. More would let one client
    // make the SFU hold (and offer others) arbitrarily many tracks.
    let m_lines: Vec<&str> = offer.lines().filter(|l| l.starts_with("m=")).collect();
    if m_lines.len() != 1 || !m_lines[0].starts_with("m=audio ") {
        return Err(bad(&format!(
            "expected exactly one audio m-line, got {} (first: {:?})",
            m_lines.len(),
            m_lines.first()
        )));
    }
    let offer = SdpOffer::from_sdp_string(offer).map_err(|e| bad(&e))?;
    let mut rtc = Rtc::builder()
        .clear_codecs()
        .enable_opus(true, false)
        .build(Instant::now());
    for addr in advertised {
        match Candidate::host(*addr, "udp") {
            Ok(c) => {
                rtc.add_local_candidate(c);
            }
            Err(e) => warn!("skipping advertised address {addr}: {e}"),
        }
    }
    let answer = rtc.sdp_api().accept_offer(offer).map_err(|e| bad(&e))?;
    let (tx, events) = tmpsc::unbounded_channel();
    let client = Client::new(
        rtc,
        room.into(),
        user.into(),
        tx,
        advertised.to_vec(),
        socket_addr.is_ipv6(),
        answer_timeout,
    );
    Ok((client, answer.to_sdp_string(), events))
}

fn poll_until_timeout(
    client: &mut Client,
    queue: &mut VecDeque<Propagated>,
    socket: &UdpSocket,
) -> Instant {
    loop {
        if !client.rtc.is_alive() {
            return Instant::now();
        }
        match client.poll_output(socket) {
            Propagated::Timeout(t) => return t,
            Propagated::Noop => {}
            p => queue.push_back(p),
        }
    }
}

/// Deliver one client's output to the other clients of its room.
fn propagate(propagated: &Propagated, clients: &mut [Client]) {
    let origin = match propagated {
        Propagated::TrackOpen(c, _) | Propagated::MediaData(c, _) => *c,
        Propagated::Noop | Propagated::Timeout(_) => return,
    };
    let Some(room) = clients
        .iter()
        .find(|c| c.id == origin)
        .map(|c| c.room.clone())
    else {
        return;
    };
    for client in clients
        .iter_mut()
        .filter(|c| c.id != origin && c.room == room)
    {
        match propagated {
            Propagated::TrackOpen(_, track) => client.handle_track_open(track.clone()),
            Propagated::MediaData(_, data) => client.handle_media_data_out(origin, data),
            Propagated::Noop | Propagated::Timeout(_) => {}
        }
    }
}
