import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connectivityCheck, summarize } from './diag';

const rep = (entries: any[]) => new Map(entries.map((e) => [e.id, e])) as unknown as RTCStatsReport;
const base = (extra: any[] = [], bytes = 0, lost = 0, recv = 0) => rep([
  { id: 't', type: 'transport', selectedCandidatePairId: 'p', iceState: 'connected' },
  { id: 'p', type: 'candidate-pair', state: 'succeeded', localCandidateId: 'l', currentRoundTripTime: 0.042 },
  { id: 'l', type: 'local-candidate', candidateType: 'srflx' },
  { id: 'c', type: 'codec', mimeType: 'audio/opus' },
  { id: 'o', type: 'outbound-rtp', kind: 'audio', codecId: 'c', bytesSent: bytes },
  { id: 'i', type: 'inbound-rtp', kind: 'audio', jitter: 0.005, packetsLost: lost, packetsReceived: recv },
  ...extra,
]);

const inb = (id: string, lost: number, recv: number) => ({ id, type: 'inbound-rtp', kind: 'audio', jitter: 0.005, packetsLost: lost, packetsReceived: recv });
const remoteIn = (f: number) => ({ id: 'r', type: 'remote-inbound-rtp', kind: 'audio', fractionLost: f });

describe('summarize', () => {
  it('computes incoming loss, kbps, rtt, jitter, codec, pair', () => {
    const d = summarize(base([], 1000, 0, 100), base([], 9000, 10, 190), 2);
    expect(d.lossDownPct).toBeCloseTo(10); // 10 lost of 100 expected
    expect(d.lossUpPct).toBeNull();
    expect(d.sendKbps).toBeCloseTo(32); // 8000 B * 8 / 2 s
    expect(d.rttMs).toBeCloseTo(42);
    expect(d.jitterMs).toBeCloseTo(5);
    expect(d).toMatchObject({ codec: 'opus', iceState: 'connected', selectedPair: 'srflx' });
  });
  it('tolerates missing fields and null prev without NaN', () => {
    const d = summarize(null, rep([]), 0);
    expect(d).toEqual({ rttMs: null, jitterMs: null, lossDownPct: null, lossUpPct: null, sendKbps: null, codec: null, iceState: 'unknown', selectedPair: null });
    const same = summarize(base([], 5, 3, 3), base([], 5, 3, 3), 1);
    expect(same.lossDownPct).toBeNull(); // zero-delta tick
    expect(same.sendKbps).toBe(0);
    expect(summarize(base(), base(), 0).sendKbps).toBeNull();
    expect(summarize(null, rep([]), 1, 'checking').iceState).toBe('checking');
  });
  it('reports outgoing loss separately and independent of incoming', () => {
    const d = summarize(null, rep([remoteIn(0.25)]), 1);
    expect(d.lossUpPct).toBeCloseTo(25);
    expect(d.lossDownPct).toBeNull();
    const both = summarize(rep([inb('a', 0, 0)]), rep([inb('a', 1, 99), remoteIn(0.5)]), 1);
    expect(both.lossDownPct).toBeCloseTo(1);
    expect(both.lossUpPct).toBeCloseTo(50);
  });
  it('sums multiple inbound streams matched by id; ignores new streams and counter resets', () => {
    const prev = rep([inb('a', 0, 100), inb('b', 5, 500), inb('gone', 0, 10)]);
    const cur = rep([inb('a', 10, 190), inb('b', 0, 10), inb('new', 99, 1)]);
    // a: +10 lost +90 recv; b reset -> 0; new has no prev -> 0
    expect(summarize(prev, cur, 1).lossDownPct).toBeCloseTo(10);
  });
  it('falls back to remote-inbound roundTripTime', () => {
    const d = summarize(null, rep([{ id: 'r', type: 'remote-inbound-rtp', kind: 'audio', roundTripTime: 0.1 }]), 1);
    expect(d.rttMs).toBeCloseTo(100);
  });
});

describe('connectivityCheck', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  function setup(opts: { state?: string; joinErr?: Error; joinDelay?: number } = {}) {
    const log: string[] = [];
    let closeCb: () => void = () => {};
    const pc: any = {
      iceConnectionState: opts.state ?? 'new',
      addEventListener: (_: string, cb: () => void) => (pc.fire = cb),
      close: () => log.push('pc'),
    };
    const track = { stop: () => log.push('track') };
    const ctx = {
      createMediaStreamDestination: () => ({ stream: { getTracks: () => [track], getAudioTracks: () => [track] } }),
      close: async () => void log.push('ctx'),
    };
    const sig = { close: () => log.push('ws'), onClose: (cb: () => void) => (closeCb = cb) };
    const deps = {
      newSignaling: () => sig as any,
      newAudioContext: () => ctx as any,
      joinRoom: async () => {
        if (opts.joinDelay) await new Promise((r) => setTimeout(r, opts.joinDelay));
        if (opts.joinErr) {
          log.push('pc'); // the real joinRoom closes its own pc on rejection
          throw opts.joinErr;
        }
        return { pc, peers: [], userId: 'u' };
      },
    };
    return { log, pc, deps, closeWs: () => closeCb() };
  }
  const cleaned = (log: string[]) => ['pc', 'track', 'ctx', 'ws'].forEach((x) => expect(log).toContain(x));

  it('fails 5 s after join naming UDP when ICE never connects', async () => {
    const s = setup();
    const p = connectivityCheck('ws://x/ws', s.deps);
    await vi.advanceTimersByTimeAsync(5000);
    const r = await p;
    expect(r.ok).toBe(false);
    expect(r.reason).toMatch(/UDP/);
    expect(r.ms).toBeGreaterThanOrEqual(5000);
    cleaned(s.log);
  });
  it('slow join then ICE connecting 1 s later is ok', async () => {
    const s = setup({ joinDelay: 4000 });
    const p = connectivityCheck('ws://x/ws', s.deps);
    await vi.advanceTimersByTimeAsync(4000);
    await vi.advanceTimersByTimeAsync(1000);
    s.pc.iceConnectionState = 'connected';
    s.pc.fire();
    const r = await p;
    expect(r.ok).toBe(true);
    expect(r.ms).toBeGreaterThanOrEqual(5000);
    cleaned(s.log);
  });
  it('slow join then no ICE reports UDP only after a full 5 s from join', async () => {
    const s = setup({ joinDelay: 4000 });
    let settled = false;
    const p = connectivityCheck('ws://x/ws', s.deps).then((r) => ((settled = true), r));
    await vi.advanceTimersByTimeAsync(8999);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    const r = await p;
    expect(r.reason).toMatch(/UDP/);
    cleaned(s.log);
  });
  it('runs the check message, not a room join, with no key or name', async () => {
    const s = setup();
    const seen: unknown[] = [];
    const join = s.deps.joinRoom;
    s.deps.joinRoom = (async (_sig: unknown, o: unknown) => (seen.push(o), join())) as any;
    const p = connectivityCheck('ws://x/ws', s.deps);
    await vi.advanceTimersByTimeAsync(5000);
    await p;
    expect(seen).toHaveLength(1);
    expect(seen[0]).toMatchObject({ check: true, bitrateKbps: 32 });
    expect(Object.keys(seen[0] as object)).not.toContain('key');
    expect(Object.keys(seen[0] as object)).not.toContain('name');
  });
  it('succeeds when ICE connects', async () => {
    const s = setup();
    const p = connectivityCheck('ws://x/ws', s.deps);
    await vi.advanceTimersByTimeAsync(300);
    s.pc.iceConnectionState = 'connected';
    s.pc.fire();
    const r = await p;
    expect(r.ok).toBe(true);
    expect(r.ms).toBeGreaterThanOrEqual(300);
    cleaned(s.log);
  });
  it('reports signaling failure without claiming UDP', async () => {
    const s = setup({ joinErr: new Error('You cannot join this room.') });
    const r = await connectivityCheck('ws://x/ws', s.deps);
    expect(r.ok).toBe(false);
    expect(r.reason).toMatch(/signaling/i);
    expect(r.reason).not.toMatch(/UDP/);
    cleaned(s.log);
  });
  it('hung join is a signaling failure at 8 s', async () => {
    const s = setup();
    s.deps.joinRoom = () => new Promise(() => {});
    const p = connectivityCheck('ws://x/ws', s.deps);
    await vi.advanceTimersByTimeAsync(8000);
    const r = await p;
    expect(r.reason).toMatch(/signaling/i);
    expect(r.reason).not.toMatch(/UDP/);
    expect(s.log).toContain('ws');
    expect(s.log).toContain('ctx');
  });
  it('ws closing mid-ICE has its own reason', async () => {
    const s = setup();
    const p = connectivityCheck('ws://x/ws', s.deps);
    await vi.advanceTimersByTimeAsync(1000);
    s.closeWs();
    const r = await p;
    expect(r.ok).toBe(false);
    expect(r.reason).toMatch(/closed/i);
    expect(r.reason).not.toMatch(/UDP|signed out/);
    cleaned(s.log);
  });
});
