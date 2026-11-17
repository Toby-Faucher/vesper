// Joins a voice room over the shared socket: the WebRTC offer and answer, renegotiation and the remote tracks.
// Its own file (not signaling.ts), so the socket in the first load does not carry the call code.
import type { Peer } from './gen/Peer';
import { tuneOpus } from './audio/opus';
import { applyBitrate } from './devices';
import type { ServerMsg } from './gen/ServerMsg';
import { SIGNAL_MESSAGES, type Signaling } from './signaling';

type Kind = 'audio' | 'video';

export async function joinRoom(
  sig: Signaling,
  opts: {
    roomId: number;
    /** Run the connectivity check (a hidden room) instead of joining `roomId`. */
    check?: boolean;
    stream: MediaStream;
    bitrateKbps: number;
    dtx?: boolean;
    onError?: (e: Error) => void;
    /** `id` is the sender's user_id: their voice and their screen share it, told apart by kind. */
    onRemoteStream?: (id: string, s: MediaStream, kind: Kind) => void;
    onRemoteStreamEnd?: (id: string, kind: Kind) => void;
  },
): Promise<{
  pc: RTCPeerConnection; peers: Peer[]; userId: string; off: () => void; video: RTCRtpSender;
  /** The call's number for `resume` (0 from an older server: no resume). */
  session: number;
  /** Moves the call's handlers (SFU offers, errors) to a new socket, for a resume. */
  rebind: (s: Signaling) => void;
}> {
  // Browsers configure their Opus encoder from the REMOTE description, and str0m's carries only
  // minptime/useinbandfec, so remote SDP is tuned before it is applied. The local offer is tuned too
  // (cheap; it states what we want to receive).
  const tune = (sdp: string) => tuneOpus(sdp, { bitrateKbps: opts.bitrateKbps, dtx: opts.dtx ?? true, fec: true });
  const report = (e: unknown) => opts.onError?.(e instanceof Error ? e : new Error(String(e)));
  const pc = new RTCPeerConnection();
  opts.stream.getAudioTracks().forEach((t) => pc.addTrack(t, opts.stream));
  // The screen share's sender, with no track: a share is replaceTrack, so it never renegotiates.
  const video = pc.addTransceiver('video', { direction: 'sendonly' }).sender;

  // Each sender's current track per kind, keyed `kind:user_id` (the SFU labels MSID with the user_id). Every new track gets
  // a stream of its own: after a rejoin Firefox reuses the MediaStream with that id and keeps the old, ended track in it.
  const live = new Map<string, MediaStreamTrack>();
  const drop = (key: string) => {
    const i = key.indexOf(':');
    if (live.delete(key)) opts.onRemoteStreamEnd?.(key.slice(i + 1), key.slice(0, i) as Kind);
  };
  pc.ontrack = (e) => {
    const t = e.track;
    const id = e.streams[0]?.id ?? t.id;
    const kind = t.kind as Kind;
    const key = `${kind}:${id}`;
    live.set(key, t);
    opts.onRemoteStream?.(id, new MediaStream([t]), kind);
    t.addEventListener('ended', () => live.get(key) === t && drop(key)); // a late end of the old track keeps the new one
  };

  let cur = sig; // the socket of the call: a resume moves it to a new one
  let chain = Promise.resolve(); // renegotiation offers must run one at a time
  const handleOffer = async (sdp: string) => {
    await pc.setRemoteDescription({ type: 'offer', sdp: tune(sdp) });
    const a = await pc.createAnswer();
    const ans = tune(a.sdp ?? '');
    await pc.setLocalDescription({ type: 'answer', sdp: ans });
    cur.send({ type: 'answer', sdp: ans });
  };

  let settled = false;
  // The socket outlives the call: off() drops this call's handlers.
  const offs: (() => void)[] = [];
  const off = () => offs.splice(0).forEach((f) => f());
  const joined = new Promise<{ answer: string; peers: Peer[]; user_id: string; session?: number }>((resolve, reject) => {
    // Before the join is answered a close fails it. After, the call outlives the socket (the caller resumes it or ends it).
    offs.push(sig.onClose(() => {
      if (settled) return;
      pc.close();
      opts.stream.getTracks().forEach((t) => t.stop());
      [...live.keys()].forEach(drop);
      settled = true;
      reject(new Error('The connection to the server closed.'));
    }));
    offs.push(sig.onMessage((m) => {
      if (m.type === 'joined') {
        settled = true;
        resolve(m);
      } else if (m.type === 'error') {
        const err = Object.assign(new Error(SIGNAL_MESSAGES[m.code] ?? 'The server refused that. Try again.'), { code: m.code });
        if (settled) report(err);
        else {
          settled = true;
          reject(err);
        }
      } else if (m.type === 'offer') {
        chain = chain.then(() => handleOffer(m.sdp)).catch(report);
      }
    }));
  });
  joined.catch(() => {}); // surfaced by the await below

  const offer = await pc.createOffer();
  const sdp = tune(offer.sdp ?? '');
  await pc.setLocalDescription({ type: 'offer', sdp });
  sig.send(opts.check ? { type: 'check', offer: sdp } : { type: 'join', room_id: opts.roomId, offer: sdp });

  const onCall = (m: ServerMsg) => {
    if (m.type === 'error') report(Object.assign(new Error(SIGNAL_MESSAGES[m.code] ?? 'The server refused that. Try again.'), { code: m.code }));
    else if (m.type === 'offer') chain = chain.then(() => handleOffer(m.sdp)).catch(report);
  };
  const rebind = (s: Signaling) => {
    off();
    cur = s;
    offs.push(s.onMessage(onCall));
  };

  try {
    const { answer, peers, user_id, session = 0 } = await joined;
    await pc.setRemoteDescription({ type: 'answer', sdp: tune(answer) });
    const audio = pc.getSenders().find((x) => x.track?.kind === 'audio');
    if (audio) {
      await applyBitrate(audio, opts.bitrateKbps).catch(report); // the SDP value is only a hint
      // Voice before a screen share on a busy link. Best effort: not every browser takes it.
      const p = audio.getParameters();
      p.encodings?.forEach((e) => (e.priority = e.networkPriority = 'high'));
      await audio.setParameters(p).catch(() => {});
    }
    return { pc, peers, userId: user_id, off, video, session, rebind };
  } catch (e) {
    off();
    pc.close();
    throw e;
  }
}
