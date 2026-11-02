import { expect, it } from 'vitest';
import { CallTally } from './call-summary';
import type { Diag } from './diag';

const diag = (p: Partial<Diag>): Diag => ({
  rttMs: null, jitterMs: null, lossDownPct: null, lossUpPct: null, sendKbps: null, codec: null, iceState: 'connected', selectedPair: null, ...p,
});
const report = (...rows: Record<string, unknown>[]) => new Map(rows.map((r, i) => [String(i), r])) as unknown as RTCStatsReport;
const inbound = (concealedSamples: number, totalSamplesReceived: number) => ({ type: 'inbound-rtp', kind: 'audio', concealedSamples, totalSamplesReceived });

it('averages the polls that measured something, and takes the concealed share from the running totals', () => {
  const t = new CallTally(0);
  t.poll(diag({ lossDownPct: 2, jitterMs: 10, rttMs: 40 }), report(inbound(10, 1000)));
  t.poll(diag({ lossDownPct: null, jitterMs: 30, rttMs: 60 }), report(inbound(30, 2000), inbound(0, 2000), { type: 'inbound-rtp', kind: 'video', concealedSamples: 9e9, totalSamplesReceived: 1 }));
  expect(t.summary('deepfilter', 90_400)).toEqual({
    duration_s: 90, mode: 'deepfilter', fallback_reason: '', loss_pct: 2, jitter_ms: 20, concealed_pct: 0.75, rtt_ms: 50, browser_ns_loading: false,
  });
});

it('a call with no numbers sends nulls, never NaN', () => {
  const s = new CallTally(0).summary('off', 500);
  expect(s).toMatchObject({ duration_s: 1, loss_pct: null, jitter_ms: null, concealed_pct: null, rtt_ms: null });
  expect(JSON.parse(JSON.stringify(s))).toEqual(s);
});

it('keeps the last fallback reason, not the loading status; the browser filter counts from 1 s of loading', () => {
  const t = new CallTally(0);
  t.mode('loading, browser filter until then', true, 0);
  t.mode('', false, 999); // ready in under a second: not counted
  expect(t.summary('deepfilter', 1000).browser_ns_loading).toBe(false);
  t.mode('loading, browser filter until then', true, 2000);
  t.mode('denoiser stalled', false, 5000);
  expect(t.summary('browser', 6000)).toMatchObject({ fallback_reason: 'denoiser stalled', browser_ns_loading: true, mode: 'browser' });
  const u = new CallTally(0);
  u.mode('loading, browser filter until then', true, 0);
  expect(u.summary('deepfilter', 1500).browser_ns_loading).toBe(true); // left while still loading
});

it('caps what the server refuses: the reason to 120 code points without control characters, the length to 24 h', () => {
  const t = new CallTally(0);
  t.mode(`denoiser error: ${'😀'.repeat(200)}\n`, false, 0);
  const s = t.summary('browser', 1e9);
  expect(Array.from(s.fallback_reason)).toHaveLength(120);
  expect(s.fallback_reason).not.toMatch(/\p{Cc}/u);
  expect(s.duration_s).toBe(86_400);
});
