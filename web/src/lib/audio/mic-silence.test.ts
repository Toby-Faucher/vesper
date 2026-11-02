import { describe, expect, it } from 'vitest';
import { MIC_QUIET, micSilence, type MicWarning, type Silence } from './mic-silence';
import type { CaptureStats } from './capture';
import type { PipelineStats } from './denoise-core';

const QUIET_ROOM = 0.0001; // -80 dBFS: a working mic in a quiet room, not silence
const LOUD = 0.05; // about -26 dBFS
type Step = {
  ms: number; rms?: number; gated?: boolean; hidden?: boolean; ageMs?: number; context?: AudioContextState; noPipeline?: boolean;
};
const stats = (s: Step): CaptureStats => ({
  pipeline: s.noPipeline ? null : ({ inputRms: s.rms ?? LOUD } as PipelineStats),
  ageMs: s.noPipeline ? null : (s.ageMs ?? 100),
  mic: { readyState: 'live', muted: false, enabled: true, settings: {}, reopens: 0 },
  context: { state: s.context ?? 'running', sampleRate: 48000 },
});
/** Polls every 100 ms through the steps; returns the warning at each poll. */
function run(steps: Step[]) {
  let s: Silence = MIC_QUIET;
  let t = 0;
  const shown: MicWarning[] = [];
  for (const step of steps)
    for (let end = t + step.ms; t < end; t += 100) {
      s = micSilence(s, t, stats(step), { gated: !!step.gated, visible: !step.hidden });
      shown.push(s.show);
    }
  return shown;
}
const never = (shown: MicWarning[]) => shown.every((w) => w === null);

describe('micSilence', () => {
  it('a quiet room (-80 dBFS) for 60 s never shows', () => {
    expect(never(run([{ ms: 60000, rms: QUIET_ROOM }]))).toBe(true);
  });

  it('digital silence: nothing before 5 s, the mic message after', () => {
    expect(never(run([{ ms: 5000, rms: 0 }]))).toBe(true);
    expect(run([{ ms: 5100, rms: 0 }]).at(-1)).toBe('mic');
    expect(run([{ ms: 5100, rms: 1e-6 }]).at(-1)).toBe('mic');
  });

  it('a brief burst of sound clears it', () => {
    const shown = run([{ ms: 6000, rms: 0 }, { ms: 100, rms: LOUD }]);
    expect(shown.at(-2)).toBe('mic');
    expect(shown.at(-1)).toBeNull();
  });

  it('never shows while muted or held off by push to talk', () => {
    expect(never(run([{ ms: 10000, rms: 0, gated: true }]))).toBe(true);
    expect(never(run([{ ms: 4000, rms: 0 }, { ms: 200, rms: 0, gated: true }, { ms: 4000, rms: 0 }]))).toBe(true); // the gate restarts the clock
  });

  it('one silent moment under 5 s never shows', () => {
    expect(never(run([{ ms: 3000 }, { ms: 4900, rms: 0 }, { ms: 3000 }]))).toBe(true);
  });

  it('no pipeline data never counts', () => {
    expect(never(run([{ ms: 10000, noPipeline: true, context: 'suspended' }]))).toBe(true);
  });

  it('a stale snapshot while visible shows the pipeline message', () => {
    expect(run([{ ms: 5100, ageMs: 2500 }]).at(-1)).toBe('pipeline');
    expect(never(run([{ ms: 10000, ageMs: 2000 }]))).toBe(true); // 2 s is still fresh
  });

  it('nothing shows while the tab is hidden', () => {
    expect(never(run([{ ms: 10000, ageMs: 9000, hidden: true }]))).toBe(true);
    expect(never(run([{ ms: 10000, rms: 0, hidden: true }]))).toBe(true);
  });

  it('a suspended context shows the pipeline message', () => {
    expect(run([{ ms: 5100, context: 'suspended' }]).at(-1)).toBe('pipeline');
  });

  it('a fresh snapshot with real input shows nothing', () => {
    expect(never(run([{ ms: 10000 }]))).toBe(true);
  });
});
