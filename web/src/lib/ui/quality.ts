import type { Diag } from '../diag';

export type Quality = 'good' | 'fair' | 'poor' | 'unknown';
type Sample = Pick<Diag, 'rttMs' | 'lossDownPct' | 'lossUpPct'>;

// Guesses: good below 150 ms and 2 % loss, fair below 300 ms and 5 %, poor otherwise.
const FAIR = { rttMs: 150, lossPct: 2 };
const POOR = { rttMs: 300, lossPct: 5 };

const loss = (d: Sample) => Math.max(d.lossDownPct ?? 0, d.lossUpPct ?? 0);

/** The call's connection quality from one stats summary. A missing value does not count against it. */
export function quality(d: Sample | null): Quality {
  if (!d || (d.rttMs === null && d.lossDownPct === null && d.lossUpPct === null)) return 'unknown';
  const rtt = d.rttMs ?? 0;
  if (rtt >= POOR.rttMs || loss(d) >= POOR.lossPct) return 'poor';
  if (rtt >= FAIR.rttMs || loss(d) >= FAIR.lossPct) return 'fair';
  return 'good';
}

/** "Weak connection: 4 % loss" or "Weak connection: 320 ms delay"; empty when good or unknown. */
export function qualityText(q: Quality, d: Sample | null): string {
  if ((q !== 'fair' && q !== 'poor') || !d) return '';
  return loss(d) >= FAIR.lossPct ? `Weak connection: ${Math.round(loss(d))} % loss` : `Weak connection: ${Math.round(d.rttMs ?? 0)} ms delay`;
}
