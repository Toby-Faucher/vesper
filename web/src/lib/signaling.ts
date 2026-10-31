import type { ClientMsg } from './gen/ClientMsg';
import type { ServerMsg } from './gen/ServerMsg';
import type { Peer } from './gen/Peer';
import { tuneOpus } from './audio/opus';
import { applyBitrate } from './devices';

export class Signaling {
  private ws: WebSocket;
  private queue: string[] = [];
  private msgCbs: ((m: ServerMsg) => void)[] = [];
  private closeCbs: (() => void)[] = [];

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
    this.ws.onclose = () => this.closeCbs.forEach((cb) => cb());
  }

  send(m: ClientMsg): void {
    const s = JSON.stringify(m);
    if (this.ws.readyState === 1) this.ws.send(s);
    else this.queue.push(s);
  }
  onMessage(cb: (m: ServerMsg) => void): void {
    this.msgCbs.push(cb);
  }
  close(): void {
    this.ws.close();
  }
  onClose(cb: () => void): void {
    this.closeCbs.push(cb);
  }
}

export async function joinRoom(
  sig: Signaling,
  opts: {
    room: string;
    name: string;
    key: string;
    stream: MediaStream;
    bitrateKbps: number;
    dtx?: boolean;
    onError?: (e: Error) => void;
    onRemoteStream?: (s: MediaStream) => void;
    onRemoteStreamEnd?: (id: string) => void;
  },
): Promise<{ pc: RTCPeerConnection; peers: Peer[]; userId: string }> {
  // Browsers configure their Opus encoder from the REMOTE description, and str0m's carries only
  // minptime/useinbandfec, so remote SDP is tuned before it is applied. The local offer is tuned too
  // (cheap; it states what we want to receive).
  const tune = (sdp: string) => tuneOpus(sdp, { bitrateKbps: opts.bitrateKbps, dtx: opts.dtx ?? true, fec: true });
  const report = (e: unknown) => opts.onError?.(e instanceof Error ? e : new Error(String(e)));
  const pc = new RTCPeerConnection();
  opts.stream.getAudioTracks().forEach((t) => pc.addTrack(t, opts.stream));

  const seen = new Set<string>();
  const drop = (id: string) => seen.delete(id) && opts.onRemoteStreamEnd?.(id);
  pc.ontrack = (e) => {
    const s = e.streams[0] ?? new MediaStream([e.track]);
    if (seen.has(s.id)) return;
    seen.add(s.id); // s.id is the sender's user_id (SFU labels MSID with it)
    opts.onRemoteStream?.(s);
    const check = () => s.getTracks().length === 0 && drop(s.id);
    s.onremovetrack = check;
    e.track.onended = check;
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
  const joined = new Promise<{ answer: string; peers: Peer[]; user_id: string }>((resolve, reject) => {
    sig.onClose(() => {
      pc.close();
      opts.stream.getTracks().forEach((t) => t.stop());
      [...seen].forEach(drop);
      if (!settled) reject(new Error('signaling closed'));
    });
    sig.onMessage((m) => {
      if (m.type === 'joined') {
        settled = true;
        resolve(m);
      } else if (m.type === 'error') {
        const err = new Error(`${m.code}: ${m.message}`);
        if (settled) report(err);
        else {
          settled = true;
          reject(err);
        }
      } else if (m.type === 'offer') {
        chain = chain.then(() => handleOffer(m.sdp)).catch(report);
      }
    });
  });
  joined.catch(() => {}); // surfaced by the await below

  const offer = await pc.createOffer();
  const sdp = tune(offer.sdp ?? '');
  await pc.setLocalDescription({ type: 'offer', sdp });
  sig.send({ type: 'join', room: opts.room, name: opts.name, key: opts.key, offer: sdp });

  try {
    const { answer, peers, user_id } = await joined;
    await pc.setRemoteDescription({ type: 'answer', sdp: tune(answer) });
    const audio = pc.getSenders().find((x) => x.track?.kind === 'audio');
    if (audio) await applyBitrate(audio, opts.bitrateKbps).catch(report); // the SDP value is only a hint
    return { pc, peers, userId: user_id };
  } catch (e) {
    pc.close();
    throw e;
  }
}
