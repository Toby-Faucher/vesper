import type { CaptureStats } from './capture';

/** Linear RMS (0..1) to dBFS, never below floor (silence would be -Infinity). */
export const dbfs = (rms: number, floor = -100) => (rms > 0 ? Math.max(floor, 20 * Math.log10(rms)) : floor);
export const fmtDb = (rms: number) => (dbfs(rms) <= -100 ? 'silent (below -100 dBFS)' : `${dbfs(rms).toFixed(1)} dBFS`);
export const fmtMs = (ms: number) => `${ms.toFixed(1)} ms`;

/** The "Audio pipeline" lines for Diagnostics and its copied report. */
export function audioRows(s: CaptureStats | null): [string, string][] {
  if (!s) return [['State', 'Not in a call']];
  const p = s.pipeline;
  const { readyState, muted, enabled } = s.mic;
  return [
    ['State', p ? (p.failure ? `${p.state}: ${p.failure}` : p.state) : 'no denoiser data (browser or off mode, or still starting)'],
    ...(p
      ? ([
        ['Input level', fmtDb(p.inputRms)],
        ['Output level', fmtDb(p.outputRms)],
        ['Queue', `${p.queuedMs.toFixed(1)} ms, ${p.inflight} frames at the denoiser`],
        ['Underruns', `${p.underruns}`],
        ['Dropped frames', `${p.droppedInput} input, ${p.droppedOut} output`],
        ['Denoiser time', `avg ${fmtMs(p.avgReplyMs)}, max ${fmtMs(p.maxReplyMs)}`],
        ['Input blocks', `${p.inputBlocks} received, ${p.emptyInputBlocks} missing`],
        ['Last report', `${((s.ageMs ?? 0) / 1000).toFixed(1)} s ago`],
      ] as [string, string][])
      : []),
    ['Mic track', `${readyState}, ${muted ? 'muted by the system' : 'not muted'}, ${enabled ? 'enabled' : 'disabled'}`],
    ['Capture context', `${s.context.state}, ${s.context.sampleRate} Hz`],
  ];
}
