import { afterEach, describe, expect, it, vi } from 'vitest';
import { joinRoom, type Signaling } from './signaling';
import type { ServerMsg } from './gen/ServerMsg';

const tick = () => new Promise((r) => setTimeout(r, 0));

class FakePC {
  static last: FakePC;
  log: string[] = [];
  active = 0;
  maxActive = 0;
  failFirst = false;
  closed = false;
  ontrack: unknown;
  constructor() { FakePC.last = this; }
  addTrack() {}
  close() { this.closed = true; }
  async createOffer() { return { type: 'offer', sdp: 'v=0' }; }
  async createAnswer() { return { type: 'answer', sdp: 'ans' }; }
  async setLocalDescription(d: { type: string; sdp: string }) { this.log.push(`local:${d.type}`); }
  async setRemoteDescription(d: { type: string; sdp: string }) {
    if (d.type !== 'offer') return;
    this.maxActive = Math.max(this.maxActive, ++this.active);
    this.log.push(`remote:${d.sdp}`);
    await tick();
    this.active--;
    if (this.failFirst && d.sdp === 'o1') throw new Error('boom');
  }
}

function setup() {
  vi.stubGlobal('RTCPeerConnection', FakePC);
  const sent: any[] = [];
  let msg: (m: ServerMsg) => void = () => {};
  let close: () => void = () => {};
  const sig = {
    send: (m: unknown) => sent.push(m),
    onMessage: (cb: (m: ServerMsg) => void) => (msg = cb),
    onClose: (cb: () => void) => (close = cb),
  } as unknown as Signaling;
  const stop = vi.fn();
  const stream = { getAudioTracks: () => [], getTracks: () => [{ stop }] } as unknown as MediaStream;
  const onError = vi.fn();
  return { sent, sig, stream, onError, stop, emit: (m: ServerMsg) => msg(m), close: () => close() };
}
const joined: ServerMsg = { type: 'joined', user_id: 'u', answer: 'a', peers: [] };
afterEach(() => vi.unstubAllGlobals());

describe('joinRoom', () => {
  it('serializes offers; failure does not block the next', async () => {
    const t = setup();
    const p = joinRoom(t.sig, { room: 'r', name: 'n', key: 'k', stream: t.stream, bitrateKbps: 64, onError: t.onError });
    await tick();
    FakePC.last.failFirst = true;
    t.emit(joined);
    await p;
    t.emit({ type: 'offer', sdp: 'o1' });
    t.emit({ type: 'offer', sdp: 'o2' });
    await tick(); await tick(); await tick();
    expect(FakePC.last.maxActive).toBe(1);
    expect(t.sent.filter((m) => m.type === 'answer')).toEqual([{ type: 'answer', sdp: 'ans' }]);
    expect(FakePC.last.log.filter((l) => l.startsWith('remote:'))).toEqual(['remote:o1', 'remote:o2']);
    expect(t.onError).toHaveBeenCalledOnce();
  });
  it('answers two good offers in order', async () => {
    const t = setup();
    const p = joinRoom(t.sig, { room: 'r', name: 'n', key: 'k', stream: t.stream, bitrateKbps: 64 });
    await tick();
    t.emit(joined);
    await p;
    t.emit({ type: 'offer', sdp: 'o1' });
    t.emit({ type: 'offer', sdp: 'o2' });
    await tick(); await tick(); await tick();
    expect(FakePC.last.maxActive).toBe(1);
    expect(t.sent.filter((m) => m.type === 'answer')).toHaveLength(2);
  });
  it('reports post-join errors and cleans up on close', async () => {
    const t = setup();
    const p = joinRoom(t.sig, { room: 'r', name: 'n', key: 'k', stream: t.stream, bitrateKbps: 64, onError: t.onError });
    await tick();
    t.emit(joined);
    await p;
    t.emit({ type: 'error', code: 'bad_message', message: 'x' });
    expect(t.onError).toHaveBeenCalledOnce();
    t.close();
    expect(FakePC.last.closed).toBe(true);
    expect(t.stop).toHaveBeenCalled();
  });
});
