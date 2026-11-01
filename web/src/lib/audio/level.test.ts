import { describe, expect, it } from 'vitest';
import { audioRows, dbfs, fmtDb, fmtMs } from './level';
import type { CaptureStats } from './capture';
import type { PipelineStats } from './denoise-core';

const pipeline: PipelineStats = {
  state: 'playing', failure: null, inputBlocks: 900, emptyInputBlocks: 3, framesSent: 240, replies: 238, underruns: 2, reprimes: 2,
  droppedInput: 0, droppedOut: 1, queuedMs: 31.4, inflight: 2, lastReplyMs: 2, avgReplyMs: 2.14, maxReplyMs: 9.5,
  inputRms: 0.1, outputRms: 0.01, inputPeak: 0.4,
};
const base: CaptureStats = {
  pipeline, ageMs: 600, mic: { readyState: 'live', muted: false, enabled: true }, context: { state: 'running', sampleRate: 48000 },
};

describe('level helpers', () => {
  it('dbfs converts linear RMS and floors silence at -100', () => {
    expect(dbfs(1)).toBe(0);
    expect(dbfs(0.1)).toBeCloseTo(-20, 9);
    expect(dbfs(0.001)).toBeCloseTo(-60, 9);
    expect(dbfs(0)).toBe(-100);
    expect(dbfs(1e-9)).toBe(-100);
    expect(dbfs(1e-9, -120)).toBeCloseTo(-120, 9);
  });

  it('formats dBFS and ms', () => {
    expect(fmtDb(0.1)).toBe('-20.0 dBFS');
    expect(fmtDb(0)).toBe('silent (below -100 dBFS)');
    expect(fmtMs(2.14)).toBe('2.1 ms');
  });

  it('audio rows: plain words for each number', () => {
    expect(audioRows(base)).toEqual([
      ['State', 'playing'],
      ['Input level', '-20.0 dBFS'],
      ['Output level', '-40.0 dBFS'],
      ['Queue', '31.4 ms, 2 frames at the denoiser'],
      ['Underruns', '2'],
      ['Dropped frames', '0 input, 1 output'],
      ['Denoiser time', 'avg 2.1 ms, max 9.5 ms'],
      ['Input blocks', '900 received, 3 missing'],
      ['Last report', '0.6 s ago'],
      ['Mic track', 'live, not muted, enabled'],
      ['Capture context', 'running, 48000 Hz'],
    ]);
  });

  it('audio rows: failure reason, no pipeline, and no capture', () => {
    const failed = audioRows({ ...base, pipeline: { ...pipeline, state: 'failed', failure: 'denoiser stalled' } });
    expect(failed[0]).toEqual(['State', 'failed: denoiser stalled']);
    const none = audioRows({ ...base, pipeline: null, ageMs: null, mic: { readyState: 'ended', muted: true, enabled: false } });
    expect(none).toEqual([
      ['State', 'no denoiser data (browser or off mode, or still starting)'],
      ['Mic track', 'ended, muted by the system, disabled'],
      ['Capture context', 'running, 48000 Hz'],
    ]);
    expect(audioRows(null)).toEqual([['State', 'Not in a call']]);
  });
});
