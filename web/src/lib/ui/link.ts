// Connection quality per person in the voice list: one sample per person per stats poll, graded, then held steady.
import { FAIR, POOR } from './quality';

/** One poll window. Null means not measured in it. Remote people get loss, jitter and concealment; you get loss and round trip. */
export type LinkSample = { lossPct: number | null; jitterMs: number | null; concealedPct: number | null; rttMs: number | null };
export type Grade = 'good' | 'fair' | 'poor';
/** `grade` is what is shown; `next` and `streak` count the windows in a row that disagree with it. */
export type Link = { grade: Grade | 'measuring'; seen: number; next: Grade | null; streak: number; last: LinkSample | null };

// Guesses, like the call bar's: jitter and concealment past these are heard as choppy speech.
export const LINK_FAIR = { lossPct: FAIR.lossPct, rttMs: FAIR.rttMs, jitterMs: 30, concealedPct: 3 };
export const LINK_POOR = { lossPct: POOR.lossPct, rttMs: POOR.rttMs, jitterMs: 60, concealedPct: 8 };
/** Fewer packets than this in a window (a quiet sender with DTX) make the loss figure too coarse to use. */
export const MIN_PACKETS = 10;
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

export const MEASURING: Link = { grade: 'measuring', seen: 0, next: null, streak: 0, last: null };

/** One window. A window without data changes nothing, so a silent or muted person keeps their last grade. */
export function stepLink(l: Link = MEASURING, s: LinkSample | null | undefined): Link {
  if (empty(s)) return l;
  const g = gradeSample(s);
  const seen = l.seen + 1;
  if (l.grade === 'measuring') return { grade: seen >= SAMPLES_TO_SHOW ? g : 'measuring', seen, next: null, streak: 0, last: s };
  if (g === l.grade) return { ...l, seen, next: null, streak: 0, last: s };
  const streak = g === l.next ? l.streak + 1 : 1;
  return streak >= SAMPLES_TO_CHANGE ? { grade: g, seen, next: null, streak: 0, last: s } : { ...l, seen, next: g, streak, last: s };
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
    if (!o) return; // the first report has no window yet
    const [lost, recv] = [delta(i, o, 'packetsLost'), delta(i, o, 'packetsReceived')];
    const [hid, silent, total] = [delta(i, o, 'concealedSamples'), delta(i, o, 'silentConcealedSamples') ?? 0, delta(i, o, 'totalSamplesReceived')];
    const jitter = num(i.jitter);
    out.set(user, {
      lossPct: lost !== null && recv !== null && lost + recv >= MIN_PACKETS ? (lost / (lost + recv)) * 100 : null,
      jitterMs: jitter === null ? null : jitter * 1000,
      // Silent concealment is the comfort noise of DTX, not a gap in speech.
      concealedPct: hid !== null && total ? (Math.max(0, hid - silent) / total) * 100 : null,
      rttMs: null,
    });
  });
  return out;
}

/** "Connection to alice: good", the accessible name of the icon. */
export function linkLabel(name: string | null, l: Link): string {
  return `${name === null ? 'Your connection' : `Connection to ${name}`}: ${l.grade}`;
}

/** The figures in plain words, for the tooltip and the person card: "1 % of audio lost, timing varies by 12 ms". */
export function linkDetail(l: Link): string {
  const s = l.last;
  if (l.grade === 'measuring' || !s) return 'Measuring, a few seconds after joining.';
  const r = (v: number) => Math.round(v);
  const parts = [
    s.lossPct !== null && `${r(s.lossPct)} % of audio lost`,
    s.jitterMs !== null && `timing varies by ${r(s.jitterMs)} ms`,
    s.concealedPct !== null && `${r(s.concealedPct)} % of sound filled in`,
    s.rttMs !== null && `${r(s.rttMs)} ms round trip`,
  ].filter(Boolean) as string[];
  const text = parts.join(', ');
  return text ? text[0].toUpperCase() + text.slice(1) + '.' : '';
}
