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

describe('summarize', () => {
  it('computes loss, kbps, rtt, jitter, codec, pair', () => {
    const d = summarize(base([], 1000, 0, 100), base([], 9000, 10, 190), 2);
    expect(d.lossPct).toBeCloseTo(10); // 10 lost of 100 expected
    expect(d.sendKbps).toBeCloseTo(32); // 8000 B * 8 / 2 s
    expect(d.rttMs).toBeCloseTo(42);
    expect(d.jitterMs).toBeCloseTo(5);
    expect(d).toMatchObject({ codec: 'opus', iceState: 'connected', selectedPair: 'srflx' });
  });
  it('tolerates missing fields and null prev without NaN', () => {
    const d = summarize(null, rep([]), 0);
    expect(d).toEqual({ rttMs: null, jitterMs: null, lossPct: null, sendKbps: null, codec: null, iceState: 'unknown', selectedPair: null });
    const same = summarize(base([], 5, 3, 3), base([], 5, 3, 3), 1);
    expect(same.lossPct).toBeNull();
    expect(same.sendKbps).toBe(0);
    expect(summarize(base(), base(), 0).sendKbps).toBeNull();
  });
  it('falls back to remote-inbound fractionLost', () => {
    const d = summarize(null, rep([{ id: 'r', type: 'remote-inbound-rtp', kind: 'audio', fractionLost: 0.25 }]), 1);
    expect(d.lossPct).toBeCloseTo(25);
  });
});

describe('connectivityCheck', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  function setup(opts: { state?: string; joinErr?: Error } = {}) {
    const log: string[] = [];
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
    const sig = { close: () => log.push('ws') };
    const deps = {
      newSignaling: () => sig as any,
      newAudioContext: () => ctx as any,
      joinRoom: async () => {
        if (opts.joinErr) throw opts.joinErr;
        return { pc, peers: [], userId: 'u' };
      },
    };
    return { log, pc, deps };
  }
  const cleaned = (log: string[]) => ['pc', 'track', 'ctx', 'ws'].forEach((x) => expect(log).toContain(x));

  it('fails after 5 s naming UDP when ICE never connects', async () => {
    const s = setup();
    const p = connectivityCheck('ws://x/ws', 'k', s.deps);
    await vi.advanceTimersByTimeAsync(5000);
    const r = await p;
    expect(r.ok).toBe(false);
    expect(r.reason).toMatch(/UDP/);
    expect(r.ms).toBeGreaterThanOrEqual(5000);
    cleaned(s.log);
  });
  it('succeeds when ICE connects', async () => {
    const s = setup();
    const p = connectivityCheck('ws://x/ws', 'k', s.deps);
    await vi.advanceTimersByTimeAsync(300);
    s.pc.iceConnectionState = 'connected';
    s.pc.fire();
    const r = await p;
    expect(r.ok).toBe(true);
    expect(r.ms).toBeGreaterThanOrEqual(300);
    cleaned(s.log);
  });
  it('reports signaling failure without claiming UDP', async () => {
    const s = setup({ joinErr: new Error('unauthorized: bad key') });
    const r = await connectivityCheck('ws://x/ws', 'k', s.deps);
    expect(r.ok).toBe(false);
    expect(r.reason).toMatch(/signaling/i);
    expect(r.reason).not.toMatch(/UDP/);
    expect(s.log).toEqual(expect.arrayContaining(['track', 'ctx', 'ws'])); // joinRoom closes its own pc on rejection
  });
  it('signaling that never answers is a signaling failure at 5 s', async () => {
    const s = setup();
    s.deps.joinRoom = () => new Promise(() => {});
    const p = connectivityCheck('ws://x/ws', 'k', s.deps);
    await vi.advanceTimersByTimeAsync(5000);
    const r = await p;
    expect(r.reason).toMatch(/signaling/i);
    expect(r.reason).not.toMatch(/UDP/);
    expect(s.log).toContain('ws');
  });
});
