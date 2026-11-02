// The summary of one call for "Your recent calls" (POST /api/me/calls at leave). In the call chunk: voice.svelte.ts feeds it
// each 2 s stats poll and the mode changes, and sends what summary() returns when the call ends.
import type { CallSummary } from './gen/CallSummary';
import type { Diag } from './diag';
import type { DenoiseMode } from './audio/mode';

const MAX_REASON = 120; // the server refuses more
const NS_LOADING_MS = 1000; // a shorter load is not counted: hardly anything was said through the browser filter

type Mean = [sum: number, n: number];
const add = (m: Mean, v: number | null) => void (v !== null && Number.isFinite(v) && v >= 0 && ((m[0] += v), m[1]++));
const mean = (m: Mean, hi: number) => (m[1] ? Math.min(hi, m[0] / m[1]) : null);

export class CallTally {
  private loss: Mean = [0, 0];
  private jitter: Mean = [0, 0];
  private rtt: Mean = [0, 0];
  private concealed: number | null = null;
  private reason = '';
  private loadingSince: number | null = null;
  private nsLoading = false;

  constructor(private startedAt: number) {}

  /** One stats poll: its summary and the raw report (for the concealed share, a running total per stream). */
  poll(d: Diag, report: RTCStatsReport) {
    add(this.loss, d.lossDownPct);
    add(this.jitter, d.jitterMs);
    add(this.rtt, d.rttMs);
    let c = 0;
    let total = 0;
    report.forEach((s: Record<string, unknown>) => {
      if (s.type !== 'inbound-rtp' || (s.kind ?? s.mediaType) !== 'audio') return;
      if (typeof s.concealedSamples === 'number' && typeof s.totalSamplesReceived === 'number') {
        c += s.concealedSamples;
        total += s.totalSamplesReceived;
      }
    });
    if (total > 0) this.concealed = Math.min(100, (100 * c) / total);
  }

  /** Each mode change. `loading` is the denoiser load with the browser filter on; any other reason is a fallback. */
  mode(reason: string, loading: boolean, now: number) {
    if (loading) this.loadingSince ??= now;
    else this.endLoading(now);
    reason = reason.replace(/(^|; )input boost applies only in deepfilter mode$/, ''); // a settings note, not a fallback
    if (reason && !loading) this.reason = reason;
  }

  private endLoading(now: number) {
    if (this.loadingSince !== null && now - this.loadingSince >= NS_LOADING_MS) this.nsLoading = true;
    this.loadingSince = null;
  }

  summary(mode: DenoiseMode, now: number): CallSummary {
    this.endLoading(now);
    return {
      duration_s: Math.max(0, Math.min(86_400, Math.round((now - this.startedAt) / 1000))),
      mode,
      fallback_reason: Array.from(this.reason.replace(/\p{Cc}/gu, ' ')).slice(0, MAX_REASON).join(''), // by code point, as the server counts
      loss_pct: mean(this.loss, 100),
      jitter_ms: mean(this.jitter, 10_000),
      concealed_pct: this.concealed,
      rtt_ms: mean(this.rtt, 60_000),
      browser_ns_loading: this.nsLoading,
    };
  }
}
