import { describe, expect, it } from 'vitest';
import { DEAD_WINDOWS, MEASURING, linkDetail, linkLabel, type Link, type LinkSample } from './link';
import { LINK_FAIR, LINK_POOR, MIN_PACKETS, gradeSample, linkSamples, ownSample, stepLink } from './link-stats';

const s = (p: Partial<LinkSample> = {}): LinkSample => ({ lossPct: null, jitterMs: null, concealedPct: null, rttMs: null, ...p });
const run = (samples: (LinkSample | null)[], from: Link = MEASURING) => samples.reduce<Link>((l, x) => stepLink(l, x), from);

describe('gradeSample', () => {
  it.each([
    ['nothing bad', s({ lossPct: 0, jitterMs: 5, concealedPct: 0 }), 'good'],
    ['loss just under fair', s({ lossPct: LINK_FAIR.lossPct - 0.01 }), 'good'],
    ['loss at fair', s({ lossPct: LINK_FAIR.lossPct }), 'fair'],
    ['loss at poor', s({ lossPct: LINK_POOR.lossPct }), 'poor'],
    ['jitter just under fair', s({ jitterMs: LINK_FAIR.jitterMs - 0.1 }), 'good'],
    ['jitter at fair', s({ jitterMs: LINK_FAIR.jitterMs }), 'fair'],
    ['jitter at poor', s({ jitterMs: LINK_POOR.jitterMs }), 'poor'],
    ['concealed at fair', s({ concealedPct: LINK_FAIR.concealedPct }), 'fair'],
    ['concealed at poor', s({ concealedPct: LINK_POOR.concealedPct }), 'poor'],
    ['round trip at fair', s({ rttMs: LINK_FAIR.rttMs }), 'fair'],
    ['round trip at poor', s({ rttMs: LINK_POOR.rttMs }), 'poor'],
    ['the worst figure counts', s({ lossPct: 0, jitterMs: LINK_POOR.jitterMs, concealedPct: 0 }), 'poor'],
    ['missing figures do not count', s({ jitterMs: 4 }), 'good'],
  ] as const)('%s', (_, sample, want) => expect(gradeSample(sample)).toBe(want));
});

describe('stepLink', () => {
  const good = s({ lossPct: 0, jitterMs: 5 });
  const fair = s({ lossPct: 3 });
  const poor = s({ lossPct: 9 });
  it('measures until enough windows have data', () => {
    expect(run([]).grade).toBe('measuring');
    expect(run([good]).grade).toBe('measuring');
    expect(run([null, s(), good]).grade).toBe('measuring'); // empty windows do not count
    expect(run([good, good]).grade).toBe('good');
  });
  it('changes only after two windows agree', () => {
    const shown = run([good, good]);
    expect(run([poor], shown).grade).toBe('good');
    expect(run([poor, poor], shown).grade).toBe('poor');
  });
  it('does not flicker on a figure that hovers at a threshold', () => {
    const shown = run([good, good]);
    const l = run([fair, good, fair, good, fair, good, fair], shown);
    expect(l.grade).toBe('good');
  });
  it('a disagreement that switches target starts the count again', () => {
    const shown = run([good, good]);
    expect(run([fair, poor], shown).grade).toBe('good');
    expect(run([fair, poor, poor], shown).grade).toBe('poor');
  });
  it('a sender with no packets for DEAD_WINDOWS goes back to measuring, unless muted; packets again start a fresh measure', () => {
    const shown = run([good, good]);
    const none = s({ jitterMs: 5, packets: 0 }); // the jitter is the stale last value
    expect(run(Array(DEAD_WINDOWS - 1).fill(none), shown).grade).toBe('good');
    const dead = run(Array(DEAD_WINDOWS).fill(none), shown);
    expect(dead.grade).toBe('measuring');
    expect(linkDetail(dead)).toBe('No audio has arrived for a few seconds.');
    const muted = [...Array(DEAD_WINDOWS + 2)].reduce<Link>((l) => stepLink(l, none, true), shown);
    expect(muted.grade).toBe('good');
    const back = run([s({ jitterMs: 5, packets: 50 })], dead);
    expect(back.grade).toBe('measuring');
    expect(linkDetail(back)).toMatch(/^Measuring/);
    expect(run([s({ jitterMs: 5, packets: 50 })], back).grade).toBe('good');
  });
  it('a window without data keeps the grade and figures', () => {
    const shown = run([poor, poor]);
    expect(run([null, s(), undefined as unknown as null], shown)).toEqual(shown);
  });
});

const rep = (entries: any[]) => new Map(entries.map((e) => [e.id, e])) as unknown as RTCStatsReport;
const inb = (id: string, track: string, x: Record<string, number>) => ({ id, type: 'inbound-rtp', kind: 'audio', trackIdentifier: track, ...x });

describe('linkSamples', () => {
  const users = new Map([['ta', 'alice'], ['tb', 'bob']]);
  it('gives each person the figures of their own track over the window', () => {
    const a0 = inb('a', 'ta', { packetsLost: 0, packetsReceived: 100, jitter: 0.01, concealedSamples: 0, silentConcealedSamples: 0, totalSamplesReceived: 0 });
    const a1 = inb('a', 'ta', { packetsLost: 5, packetsReceived: 195, jitter: 0.012, concealedSamples: 4800, silentConcealedSamples: 2400, totalSamplesReceived: 48000 });
    const b0 = inb('b', 'tb', { packetsLost: 0, packetsReceived: 0, jitter: 0.002 });
    const b1 = inb('b', 'tb', { packetsLost: 0, packetsReceived: 100, jitter: 0.002 });
    const out = linkSamples(rep([a0, b0]), rep([a1, b1, { id: 'o', type: 'outbound-rtp', kind: 'audio' }]), users);
    expect(out.get('alice')!.lossPct).toBeCloseTo(5); // 5 lost of 100 expected
    expect(out.get('alice')!.jitterMs).toBeCloseTo(12);
    expect(out.get('alice')!.concealedPct).toBeCloseTo(5); // 2400 not silent of 48000
    expect(out.get('bob')).toEqual({ lossPct: 0, jitterMs: 2, concealedPct: null, rttMs: null, packets: 100 });
  });
  it('no window yet, an unknown track, too few packets or a counter reset give no figure', () => {
    const cur = rep([inb('a', 'ta', { packetsLost: 0, packetsReceived: 105 }), inb('x', 'tx', { packetsLost: 0, packetsReceived: 50 })]);
    expect(linkSamples(null, cur, users).size).toBe(0);
    const out = linkSamples(rep([inb('a', 'ta', { packetsLost: 0, packetsReceived: 100 })]), cur, users);
    expect([...out.keys()]).toEqual(['alice']);
    expect(out.get('alice')!.lossPct).toBeNull(); // 5 packets: under MIN_PACKETS
    const reset = linkSamples(rep([inb('a', 'ta', { packetsLost: 9, packetsReceived: 900 })]), cur, users);
    expect(reset.get('alice')!.lossPct).toBeNull();
    for (const v of Object.values(reset.get('alice')!)) expect(Number.isNaN(v)).toBe(false);
  });
});

describe('DTX silence, a rejoin and your own row', () => {
  const users = new Map([['ta', 'alice'], ['tb', 'alice']]);
  it('a quiet DTX sender (a packet every 400 ms, clean) is graded good, not poor', () => {
    const w = (n: number) => inb('a', 'ta', { packetsLost: 0, packetsReceived: n, jitter: 0.004, concealedSamples: 1000 * n, silentConcealedSamples: 1000 * n, totalSamplesReceived: 960 * n * 100 });
    let l = MEASURING;
    for (let n = 1; n < 5; n++) l = stepLink(l, linkSamples(rep([w(n * 5)]), rep([w(n * 5 + 5)]), users).get('alice'));
    expect(l.grade).toBe('good');
    expect(l.last!.lossPct).toBeNull(); // 5 packets: below MIN_PACKETS
    expect(MIN_PACKETS).toBeGreaterThanOrEqual(25);
  });
  it('a rejoin (a new track and stats id) gives no figure in its first window', () => {
    const old = inb('a', 'ta', { packetsLost: 0, packetsReceived: 500 });
    const fresh = inb('b', 'tb', { packetsLost: 0, packetsReceived: 40 });
    expect(linkSamples(rep([old]), rep([fresh]), users).size).toBe(0);
    expect(linkSamples(rep([fresh]), rep([inb('b', 'tb', { packetsLost: 0, packetsReceived: 90 })]), users).get('alice')!.lossPct).toBe(0);
  });
  const diag = (p: Partial<Parameters<typeof ownSample>[0]>) => ({ rttMs: 40, jitterMs: null, lossDownPct: 1, lossUpPct: 0, sendKbps: null, codec: null, iceState: 'connected', selectedPair: null, ...p });
  it('your own row: the worse loss and the round trip; all lost while ICE is down', () => {
    expect(ownSample(diag({}))).toEqual({ lossPct: 1, rttMs: 40, jitterMs: null, concealedPct: null });
    expect(ownSample(diag({ lossDownPct: null, lossUpPct: null })).lossPct).toBeNull();
    for (const iceState of ['disconnected', 'failed']) expect(gradeSample(ownSample(diag({ iceState })))).toBe('poor');
  });
});

describe('text', () => {
  it('names the person and the grade', () => {
    expect(linkLabel('alice', run([s({ lossPct: 0 }), s({ lossPct: 0 })]))).toBe('Connection to alice: good');
    expect(linkLabel(null, MEASURING)).toBe('Your connection: measuring');
  });
  it('says the figures in plain words', () => {
    const l = run([s({ lossPct: 0 }), s({ lossPct: 1.4, jitterMs: 12.2, concealedPct: 0.4 })]);
    expect(linkDetail(l)).toBe('1 % of audio lost, timing varies by 12 ms, 0 % of sound filled in.');
    expect(linkDetail(run([s({ rttMs: 40 }), s({ rttMs: 41 })]))).toBe('41 ms round trip.');
    expect(linkDetail(MEASURING)).toMatch(/^Measuring/);
  });
});
