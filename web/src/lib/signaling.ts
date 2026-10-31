import type { ClientMsg } from './gen/ClientMsg';
import type { ServerMsg } from './gen/ServerMsg';
import type { Peer } from './gen/Peer';
import { tuneOpus } from './audio/opus';

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
  onClose(cb: () => void): void {
    this.closeCbs.push(cb);
  }
}

export async function joinRoom(
  sig: Signaling,
  opts: { room: string; name: string; key: string; stream: MediaStream; bitrateKbps: number },
): Promise<{ pc: RTCPeerConnection; peers: Peer[] }> {
  const tune = (sdp: string) => tuneOpus(sdp, { bitrateKbps: opts.bitrateKbps, dtx: true, fec: true });
  const pc = new RTCPeerConnection();
  opts.stream.getAudioTracks().forEach((t) => pc.addTrack(t, opts.stream));

  const joined = new Promise<{ answer: string; peers: Peer[] }>((resolve, reject) => {
    sig.onClose(() => reject(new Error('signaling closed')));
    sig.onMessage((m) => {
      if (m.type === 'joined') resolve(m);
      else if (m.type === 'error') reject(new Error(`${m.code}: ${m.message}`));
      else if (m.type === 'offer') {
        // SFU-initiated renegotiation
        void (async () => {
          await pc.setRemoteDescription({ type: 'offer', sdp: m.sdp });
          const a = await pc.createAnswer();
          const sdp = tune(a.sdp ?? '');
          await pc.setLocalDescription({ type: 'answer', sdp });
          sig.send({ type: 'answer', sdp });
        })();
      }
    });
  });
  joined.catch(() => {}); // surfaced by the await below

  const offer = await pc.createOffer();
  const sdp = tune(offer.sdp ?? '');
  await pc.setLocalDescription({ type: 'offer', sdp });
  sig.send({ type: 'join', room: opts.room, name: opts.name, key: opts.key, offer: sdp });

  try {
    const { answer, peers } = await joined;
    await pc.setRemoteDescription({ type: 'answer', sdp: answer });
    return { pc, peers };
  } catch (e) {
    pc.close();
    throw e;
  }
}
