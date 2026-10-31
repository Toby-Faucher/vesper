import { describe, expect, it } from 'vitest';
import { MIC_QUIET, micSilence, type Silence } from './mic-silence';

const QUIET = 0.0001; // -80 dBFS
const LOUD = 0.05; // about -26 dBFS
type Step = [ms: number, rms: number | null, gated?: boolean];
/** Polls every 100 ms through the steps; returns show at each poll. */
function run(steps: Step[]) {
  let s: Silence = MIC_QUIET;
  let t = 0;
  const shown: boolean[] = [];
  for (const [ms, rms, gated = false] of steps)
    for (let end = t + ms; t < end; t += 100) {
      s = micSilence(s, t, rms, gated);
      shown.push(s.show);
    }
  return shown;
}

describe('micSilence', () => {
  it('never shows before 5 s of silence', () => {
    expect(run([[5000, QUIET]]).some(Boolean)).toBe(false);
  });

  it('shows after 5 s of silence', () => {
    expect(run([[5100, QUIET]]).at(-1)).toBe(true);
    expect(run([[5100, 0]]).at(-1)).toBe(true); // no input at all reads as 0
  });

  it('does not show while muted or held off by push to talk', () => {
    expect(run([[10000, QUIET, true]]).some(Boolean)).toBe(false);
    expect(run([[4000, QUIET], [200, QUIET, true], [4000, QUIET]]).some(Boolean)).toBe(false); // the gate restarts the clock
  });

  it('clears when sound returns', () => {
    const shown = run([[6000, QUIET], [100, LOUD]]);
    expect(shown.at(-2)).toBe(true);
    expect(shown.at(-1)).toBe(false);
  });

  it('ignores one brief quiet moment', () => {
    expect(run([[3000, LOUD], [1000, QUIET], [3000, LOUD]]).some(Boolean)).toBe(false);
    expect(run([[4000, QUIET], [100, LOUD], [4000, QUIET]]).some(Boolean)).toBe(false);
  });

  it('no pipeline data never counts as silence', () => {
    expect(run([[10000, null]]).some(Boolean)).toBe(false);
  });
});
