import { describe, expect, it, vi } from 'vitest';
import { Signaling } from './signaling';

class FakeWS {
  static last: FakeWS;
  static OPEN = 1;
  readyState = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(public url: string) { FakeWS.last = this; }
  send(s: string) { this.sent.push(s); }
  open() { this.readyState = 1; this.onopen?.(); }
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
    s.send({ type: 'set_muted', muted: true });
    expect(ws.sent.map((x) => JSON.parse(x))).toEqual([
      { type: 'leave' }, { type: 'set_muted', muted: true },
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
  it('calls onClose', () => {
    const { s, ws } = make();
    const cb = vi.fn();
    s.onClose(cb);
    ws.onclose!();
    expect(cb).toHaveBeenCalledOnce();
  });
});
