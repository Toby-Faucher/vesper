import { afterEach, describe, expect, it, vi } from 'vitest';
import { Signaling } from './signaling';
import { joinRoom } from './join';

class FakeWS {
  static last: FakeWS;
  static OPEN = 1;
  readyState = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: ((e: { code: number }) => void) | null = null;
  constructor(public url: string) { FakeWS.last = this; }
  send(s: string) { this.sent.push(s); }
  open() { this.readyState = 1; this.onopen?.(); }
  closed = false;
  close() { this.closed = true; this.onclose?.({ code: 1000 }); }
}
const make = () => {
  const s = new Signaling('ws://x/ws', FakeWS as unknown as typeof WebSocket);
  return { s, ws: FakeWS.last };
};

describe('Signaling', () => {
  it('sends JSON, queueing until open', () => {
    const { s, ws } = make();
    s.send({ type: 'leave' });
    expect(ws.sent).toEqual([]);
    ws.open();
    s.send({ type: 'set_voice_state', muted: true, deafened: false });
    expect(ws.sent.map((x) => JSON.parse(x))).toEqual([
      { type: 'leave' }, { type: 'set_voice_state', muted: true, deafened: false },
    ]);
    expect(ws.url).toBe('ws://x/ws');
  });
  it('parses incoming messages and ignores invalid JSON', () => {
    const { s, ws } = make();
    const cb = vi.fn();
    s.onMessage(cb);
    ws.onmessage!({ data: '{"type":"peer_left","user_id":"u1"}' });
    expect(() => ws.onmessage!({ data: 'not json' })).not.toThrow();
    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenCalledWith({ type: 'peer_left', user_id: 'u1' });
  });
  it('calls onClose with the close code', () => {
    const { s, ws } = make();
    const cb = vi.fn();
    s.onClose(cb);
    ws.onclose!({ code: 4001 });
    expect(cb).toHaveBeenCalledOnce();
    expect(cb).toHaveBeenCalledWith(false, 4001); // closed by the server
  });
  it('close() reports the close as ours', () => {
    const { s, ws } = make();
    const cb = vi.fn();
    s.onClose(cb);
    s.close();
    expect(cb).toHaveBeenCalledWith(true, 1000);
  });
  it('leave_keeps_socket_open', () => {
    const { s, ws } = make();
    ws.open();
    const cb = vi.fn();
    s.onClose(cb);
    s.leave();
    expect(ws.sent.map((x) => JSON.parse(x))).toEqual([{ type: 'leave' }]);
    expect(ws.closed).toBe(false);
    expect(cb).not.toHaveBeenCalled();
  });
  it('unsubscribe_stops_callbacks', () => {
    const { s, ws } = make();
    const [m1, m2, c1] = [vi.fn(), vi.fn(), vi.fn()];
    const off1 = s.onMessage(m1);
    s.onMessage(m2);
    const offC = s.onClose(c1);
    ws.onmessage!({ data: '{"type":"leave"}' });
    off1();
    offC();
    ws.onmessage!({ data: '{"type":"leave"}' });
    ws.onclose!({ code: 1006 });
    expect(m1).toHaveBeenCalledOnce();
    expect(m2).toHaveBeenCalledTimes(2);
    expect(c1).not.toHaveBeenCalled();
  });
  it('a callback may unsubscribe another during dispatch without skipping it', () => {
    const { s, ws } = make();
    const b = vi.fn();
    let offB = () => {};
    s.onMessage(() => offB());
    offB = s.onMessage(b);
    ws.onmessage!({ data: '{"type":"leave"}' });
    expect(b).toHaveBeenCalledOnce();
  });
});

class FakePC {
  getSenders() { return []; }
  addTrack() {}
  close() {}
  async createOffer() { return { type: 'offer', sdp: 'v=0' }; }
  async setLocalDescription() {}
  async setRemoteDescription() {}
}
const stream = { getAudioTracks: () => [], getTracks: () => [] } as unknown as MediaStream;
const tick = () => new Promise((r) => setTimeout(r, 0));

describe('joinRoom messages', () => {
  afterEach(() => vi.unstubAllGlobals());
  for (const check of [false, true]) {
    it(check ? 'check_sends_check_message' : 'join_sends_room_id_and_no_key', async () => {
      vi.stubGlobal('RTCPeerConnection', FakePC);
      const { s, ws } = make();
      ws.open();
      const p = joinRoom(s, { roomId: 7, check, stream, bitrateKbps: 64 });
      await tick();
      const sent = JSON.parse(ws.sent[0]);
      if (check) expect(Object.keys(sent).sort()).toEqual(['offer', 'type']);
      else expect(Object.keys(sent).sort()).toEqual(['offer', 'room_id', 'type']);
      expect(sent).toMatchObject(check ? { type: 'check' } : { type: 'join', room_id: 7 });
      ws.onmessage!({ data: JSON.stringify({ type: 'joined', user_id: 'u', answer: 'a', peers: [] }) });
      const r = await p;
      r.off();
      const later = vi.fn();
      s.onMessage(later);
      ws.onmessage!({ data: JSON.stringify({ type: 'error', code: 'bad_message', message: 'x' }) });
      expect(later).toHaveBeenCalledOnce(); // the socket lives on after off()
    });
  }
  it('a refused join rejects with plain text, not the error code', async () => {
    vi.stubGlobal('RTCPeerConnection', FakePC);
    const { s, ws } = make();
    ws.open();
    const p = joinRoom(s, { roomId: 1, stream, bitrateKbps: 64 });
    await tick();
    ws.onmessage!({ data: JSON.stringify({ type: 'error', code: 'room_full', message: 'room is full' }) });
    await expect(p).rejects.toThrow('This room is full.');
  });
  it('an unknown error code shows a plain sentence, not the server text', async () => {
    vi.stubGlobal('RTCPeerConnection', FakePC);
    const { s, ws } = make();
    ws.open();
    const p = joinRoom(s, { roomId: 1, stream, bitrateKbps: 64 });
    await tick();
    ws.onmessage!({ data: JSON.stringify({ type: 'error', code: 'new_code', message: 'raw server text' }) });
    await expect(p).rejects.toThrow('The server refused that. Try again.');
  });
});
