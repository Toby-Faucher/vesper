// The stats math behind the connection icons: per-person samples from getStats, a grade, and the hold that stops flicker.
import type { Diag } from '../diag';
import { DEAD_WINDOWS, MEASURING, type Grade, type Link, type LinkSample } from './link';
import { FAIR, POOR } from './quality';

// Guesses, like the call bar's: jitter and concealment past these are heard as choppy speech.
export const LINK_FAIR = { lossPct: FAIR.lossPct, rttMs: FAIR.rttMs, jitterMs: 30, concealedPct: 3 };
export const LINK_POOR = { lossPct: POOR.lossPct, rttMs: POOR.rttMs, jitterMs: 60, concealedPct: 8 };
/** Fewer packets than this in a window (a quiet sender with DTX) make the loss figure too coarse: 1 lost of 10 is 10 %. */
export const MIN_PACKETS = 25;
/** Windows with data before the first grade shows. */
export const SAMPLES_TO_SHOW = 2;
/** Windows in a row that must agree on a new grade before it replaces the shown one. */
export const SAMPLES_TO_CHANGE = 2;

const KEYS = ['lossPct', 'jitterMs', 'concealedPct', 'rttMs'] as const;
const over = (s: LinkSample, t: typeof LINK_FAIR) => KEYS.some((k) => (s[k] ?? 0) >= t[k]);
const empty = (s: LinkSample | null | undefined): s is null | undefined => !s || KEYS.every((k) => s[k] === null);

export function gradeSample(s: LinkSample): Grade {
  return over(s, LINK_POOR) ? 'poor' : over(s, LINK_FAIR) ? 'fair' : 'good';
}

/**
 * One window. A window without data changes nothing, so a quiet person keeps their last grade. A window with no packet at all
 * says nothing about the line either (its jitter is the last one), but after DEAD_WINDOWS of them from someone who is not
 * `muted` the sender is gone or frozen: the grade goes back to measuring until packets come again.
 */
export function stepLink(l: Link = MEASURING, s: LinkSample | null | undefined, muted = false): Link {
  if (s && s.packets === 0) {
    const idle = l.idle + 1;
    return idle >= DEAD_WINDOWS && !muted ? { ...MEASURING, idle, last: l.last } : { ...l, idle };
  }
  if (empty(s)) return l;
  const g = gradeSample(s);
  const seen = l.seen + 1;
  if (l.grade === 'measuring') return { grade: seen >= SAMPLES_TO_SHOW ? g : 'measuring', seen, next: null, streak: 0, idle: 0, last: s };
  if (g === l.grade) return { ...l, seen, next: null, streak: 0, idle: 0, last: s };
  const streak = g === l.next ? l.streak + 1 : 1;
  return streak >= SAMPLES_TO_CHANGE ? { grade: g, seen, next: null, streak: 0, idle: 0, last: s } : { ...l, seen, next: g, streak, idle: 0, last: s };
}

/** Your own row from the call's summary. With ICE disconnected or failed nothing gets through, so it counts as all lost. */
export function ownSample(d: Diag): LinkSample {
  if (d.iceState === 'disconnected' || d.iceState === 'failed') return { lossPct: 100, rttMs: null, jitterMs: null, concealedPct: null };
  const lossPct = d.lossDownPct === null && d.lossUpPct === null ? null : Math.max(d.lossDownPct ?? 0, d.lossUpPct ?? 0);
  return { lossPct, rttMs: d.rttMs, jitterMs: null, concealedPct: null };
}

type Stat = Record<string, any>;
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const delta = (a: Stat, b: Stat | undefined, k: string) => {
  const [x, y] = [num(a[k]), num(b?.[k])];
  return x === null || y === null || x < y ? null : x - y; // a counter reset gives no figure
};

/** Per person from one getStats report and the one before it. `users` maps a remote track id to its user_id. */
export function linkSamples(prev: RTCStatsReport | null, cur: RTCStatsReport, users: Map<string, string>): Map<string, LinkSample> {
  const before = new Map<string, Stat>();
  prev?.forEach((s: Stat) => before.set(s.id, s));
  const out = new Map<string, LinkSample>();
  cur.forEach((i: Stat) => {
    const user = users.get(i.trackIdentifier);
    if (i.type !== 'inbound-rtp' || (i.kind ?? i.mediaType) !== 'audio' || !user) return;
    const o = before.get(i.id);
    if (!o) return; // the first report of a track (also after a rejoin) has no window yet
    const [lost, recv] = [delta(i, o, 'packetsLost'), delta(i, o, 'packetsReceived')];
    const [hid, silent, total] = [delta(i, o, 'concealedSamples'), delta(i, o, 'silentConcealedSamples') ?? 0, delta(i, o, 'totalSamplesReceived')];
    const jitter = num(i.jitter);
    out.set(user, {
      lossPct: lost !== null && recv !== null && lost + recv >= MIN_PACKETS ? (lost / (lost + recv)) * 100 : null,
      jitterMs: jitter === null ? null : jitter * 1000,
      // Silent concealment is the comfort noise of DTX, not a gap in speech.
      concealedPct: hid !== null && total ? (Math.max(0, hid - silent) / total) * 100 : null,
      rttMs: null,
      packets: recv,
    });
  });
  return out;
}
