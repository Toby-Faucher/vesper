import type { ClientMsg } from './gen/ClientMsg';
import type { ServerMsg } from './gen/ServerMsg';
import type { Peer } from './gen/Peer';
import type { ErrorCode } from './gen/ErrorCode';
import { tuneOpus } from './audio/opus';
import { applyBitrate } from './devices';

/** Plain text for the server's socket error codes. */
export const SIGNAL_MESSAGES: Record<ErrorCode, string> = {
  room_full: 'This room is full.',
  forbidden: 'You cannot join this room.',
  bad_message: 'The server did not accept that. Reload the page and try again.',
  rate_limited: 'Too many tries. Wait a moment and try again.',
};

export class Signaling {
  private ws: WebSocket;
  private queue: string[] = [];
  private msgCbs: ((m: ServerMsg) => void)[] = [];
  private closeCbs: ((byClient: boolean, code: number) => void)[] = [];
  private byClient = false;

  constructor(url: string, ws: typeof WebSocket = WebSocket) {
    this.ws = new ws(url);
    this.ws.onopen = () => this.queue.splice(0).forEach((s) => this.ws.send(s));
    this.ws.onmessage = (e) => {
      let m: ServerMsg;
      try {
        m = JSON.parse(e.data);
      } catch {
        return;
      }
      this.msgCbs.forEach((cb) => cb(m));
    };
    this.ws.onclose = (e) => this.closeCbs.forEach((cb) => cb(this.byClient, e?.code ?? 1006));
  }

  send(m: ClientMsg): void {
    const s = JSON.stringify(m);
    if (this.ws.readyState === 1) this.ws.send(s);
    else this.queue.push(s);
  }
  /** Returns a function that removes the callback. */
  onMessage(cb: (m: ServerMsg) => void): () => void {
    this.msgCbs.push(cb);
    return () => (this.msgCbs = this.msgCbs.filter((x) => x !== cb));
  }
  close(): void {
    this.byClient = true;
    this.ws.close();
  }
  /** Ends the call. The socket stays open. */
  leave(): void {
    this.send({ type: 'leave' });
  }
  /**
   * `byClient` is true when close() caused it, false for a server or network close.
   * `code` is the close code (4001 session ended, 4003 moderator disconnect). Returns an unsubscribe function.
   */
  onClose(cb: (byClient: boolean, code: number) => void): () => void {
    this.closeCbs.push(cb);
    return () => (this.closeCbs = this.closeCbs.filter((x) => x !== cb));
  }
}

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
    /** `id` is the sender's user_id. */
    onRemoteStream?: (id: string, s: MediaStream) => void;
    onRemoteStreamEnd?: (id: string) => void;
  },
): Promise<{ pc: RTCPeerConnection; peers: Peer[]; userId: string; off: () => void }> {
  // Browsers configure their Opus encoder from the REMOTE description, and str0m's carries only
  // minptime/useinbandfec, so remote SDP is tuned before it is applied. The local offer is tuned too
  // (cheap; it states what we want to receive).
  const tune = (sdp: string) => tuneOpus(sdp, { bitrateKbps: opts.bitrateKbps, dtx: opts.dtx ?? true, fec: true });
  const report = (e: unknown) => opts.onError?.(e instanceof Error ? e : new Error(String(e)));
  const pc = new RTCPeerConnection();
  opts.stream.getAudioTracks().forEach((t) => pc.addTrack(t, opts.stream));

  // Each sender's current track, keyed on its user_id (the SFU labels MSID with it). Every new track gets a stream of
  // its own: after a rejoin Firefox reuses the MediaStream with that id and keeps the old, ended track in it.
  const live = new Map<string, MediaStreamTrack>();
  const drop = (id: string) => live.delete(id) && opts.onRemoteStreamEnd?.(id);
  pc.ontrack = (e) => {
    const id = e.streams[0]?.id ?? e.track.id;
    const t = e.track;
    live.set(id, t);
    opts.onRemoteStream?.(id, new MediaStream([t]));
    t.addEventListener('ended', () => live.get(id) === t && drop(id)); // a late end of the old track keeps the new one
  };

  let chain = Promise.resolve(); // renegotiation offers must run one at a time
  const handleOffer = async (sdp: string) => {
    await pc.setRemoteDescription({ type: 'offer', sdp: tune(sdp) });
    const a = await pc.createAnswer();
    const ans = tune(a.sdp ?? '');
    await pc.setLocalDescription({ type: 'answer', sdp: ans });
    sig.send({ type: 'answer', sdp: ans });
  };

  let settled = false;
  // The socket outlives the call: off() drops this call's handlers.
  const offs: (() => void)[] = [];
  const off = () => offs.splice(0).forEach((f) => f());
  const joined = new Promise<{ answer: string; peers: Peer[]; user_id: string }>((resolve, reject) => {
    offs.push(sig.onClose(() => {
      pc.close();
      opts.stream.getTracks().forEach((t) => t.stop());
      [...live.keys()].forEach(drop);
      if (!settled) reject(new Error('The connection to the server closed.'));
    }));
    offs.push(sig.onMessage((m) => {
      if (m.type === 'joined') {
        settled = true;
        resolve(m);
      } else if (m.type === 'error') {
        const err = new Error(SIGNAL_MESSAGES[m.code] ?? 'The server refused that. Try again.');
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

  try {
    const { answer, peers, user_id } = await joined;
    await pc.setRemoteDescription({ type: 'answer', sdp: tune(answer) });
    const audio = pc.getSenders().find((x) => x.track?.kind === 'audio');
    if (audio) await applyBitrate(audio, opts.bitrateKbps).catch(report); // the SDP value is only a hint
    return { pc, peers, userId: user_id, off };
  } catch (e) {
    off();
    pc.close();
    throw e;
  }
}
