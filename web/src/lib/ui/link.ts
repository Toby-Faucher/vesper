// Connection quality per person in the voice list: the shapes and the words. The stats math is in link-stats.ts, loaded at
// the first join, so the first page load does not carry it.

/** One poll window. Null means not measured in it. Remote people get loss, jitter and concealment; you get loss and round trip.
 * `packets` is how many arrived in the window (null for your own row). */
export type LinkSample = { lossPct: number | null; jitterMs: number | null; concealedPct: number | null; rttMs: number | null; packets?: number | null };
export type Grade = 'good' | 'fair' | 'poor';
/** `grade` is what is shown; `next` and `streak` count the windows in a row that disagree with it; `idle` counts windows in a
 * row in which no packet arrived. */
export type Link = { grade: Grade | 'measuring'; seen: number; next: Grade | null; streak: number; idle: number; last: LinkSample | null };

export const MEASURING: Link = { grade: 'measuring', seen: 0, next: null, streak: 0, idle: 0, last: null };
/** Windows in a row with no packet from someone who is not muted before their grade goes back to measuring (6 s). */
export const DEAD_WINDOWS = 3;

/** "Connection to alice: good", the accessible name of the icon. */
export function linkLabel(name: string | null, l: Link): string {
  return `${name === null ? 'Your connection' : `Connection to ${name}`}: ${l.grade}`;
}

/** The figures in plain words, for the tooltip and the person card: "1 % of audio lost, timing varies by 12 ms". */
export function linkDetail(l: Link): string {
  const s = l.last;
  if (l.idle >= DEAD_WINDOWS) return 'No audio has arrived for a few seconds.';
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
