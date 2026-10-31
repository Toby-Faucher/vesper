import type { CaptureStats } from './capture';

const SILENT_RMS = 1e-5; // about -100 dBFS: digital silence (no input, or a hardware mute). A real mic's own noise is far above it.
const STALE_MS = 2000; // the worklet reports every 500 ms
const SILENT_MS = 5000;

/** 'mic': the mic sends digital silence. 'pipeline': the worklet stopped reporting, or its context is not running. */
export type MicWarning = 'mic' | 'pipeline' | null;
export type Silence = { since: number | null; show: MicWarning };
export const MIC_QUIET: Silence = { since: null, show: null };

/**
 * One poll of the "no sound" check. Judged on the raw input level only. No pipeline data (browser or off mode) never
 * counts. gated is mute or PTT. A hidden tab shows nothing: timers and messages are throttled there.
 */
export function micSilence(prev: Silence, now: number, s: CaptureStats | null, o: { gated: boolean; visible: boolean }): Silence {
  const p = s?.pipeline;
  const kind: MicWarning = o.gated || !o.visible || !p ? null
    : s!.context.state !== 'running' || (s!.ageMs ?? 0) > STALE_MS ? 'pipeline'
    : p.inputRms < SILENT_RMS ? 'mic' : null;
  const since = kind ? (prev.since ?? now) : null;
  return { since, show: since !== null && now - since >= SILENT_MS ? kind : null };
}
