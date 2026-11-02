// Settings > Account password rules, outside the component so vitest can load them without the page.
import { ApiFailure, errorText } from '../../lib/api';
import { UI_MESSAGES as M } from '../../lib/ui/messages';
import type { CallRecord } from '../../lib/gen/CallRecord';
export type PwError = { at: 'old' | 'new' | 'confirm' | 'form'; text: string };
/** The checks before any request: the length (in characters, as the server counts) and the two new passwords match. */
export function pwCheck(newPw: string, confirmPw: string): PwError | null {
  const n = [...newPw].length;
  if (n < 8 || n > 128) return { at: 'new', text: M.passwordShort };
  return newPw === confirmPw ? null : { at: 'confirm', text: M.passwordMismatch };
}
/** A failed change. The server sends 403 forbidden for a wrong current password and for a request from another
 *  origin; only its message tells them apart. */
export const pwFailure = (err: unknown): PwError =>
  err instanceof ApiFailure && err.code === 'forbidden' && err.detail === 'wrong password' ? { at: 'old', text: M.passwordWrong } : { at: 'form', text: errorText(err) };


const fig = (v: number | null, unit: string, digits = 0) => (v === null ? '–' : `${v.toFixed(digits)} ${unit}`);
const MODE = { deepfilter: 'DeepFilterNet', browser: 'browser filter', off: 'no filter' } as const;
/** One line of Your recent calls: how long, the filter it ended on (and why), and the figures. A figure never measured is a dash. */
export function callLine(c: CallRecord): { head: string; figures: string } {
  const min = Math.round(c.duration_s / 60);
  const length = c.duration_s < 60 ? `${c.duration_s} s` : min < 60 ? `${min} min` : `${Math.floor(min / 60)} h ${min % 60} min`;
  const why = c.fallback_reason ? ` (${c.fallback_reason})` : '';
  const loading = c.browser_ns_loading ? ', browser filter while it loaded' : '';
  return {
    head: `${length}, ${MODE[c.mode]}${why}${loading}`,
    figures: `Loss ${fig(c.loss_pct, '%', 1)}, jitter ${fig(c.jitter_ms, 'ms')}, round trip ${fig(c.rtt_ms, 'ms')}, concealed ${fig(c.concealed_pct, '%', 1)}`,
  };
}
