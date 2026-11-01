import { afterEach, expect, it, vi } from 'vitest';
import { MAX_GAIN, TONES, chime } from './chime';
import type { Cue } from './cues';

// A stand-in AudioContext that records what chime() does with it.
function fakeAudio(state: AudioContextState, closeFails = false) {
  const made: { resumed: number; closed: number; tones: number }[] = [];
  class Ctx {
    state = state;
    currentTime = 0;
    destination = {};
    rec = { resumed: 0, closed: 0, tones: 0 };
    constructor() { made.push(this.rec); (globalThis as unknown as { lastCtx: unknown }).lastCtx = this; }
    resume() { this.rec.resumed++; return Promise.resolve(); }
    close() { this.rec.closed++; return closeFails ? Promise.reject(new Error('closed already')) : Promise.resolve(); }
    sinks: string[] = [];
    oscs: { type?: string; frequency: { value?: number }; start?: number; stop?: number }[] = [];
    peaks: number[] = [];
    setSinkId(id: string) { this.sinks.push(id); return Promise.resolve(); }
    createOscillator() {
      const r = this.rec;
      const o: Ctx['oscs'][number] = { frequency: {} };
      this.oscs.push(o);
      return Object.assign(o, { connect: (n: unknown) => n, start: (t: number) => { r.tones++; o.start = t; }, stop: (t: number) => (o.stop = t) });
    }
    createGain() {
      const peaks = this.peaks;
      return { gain: { setValueAtTime() {}, linearRampToValueAtTime: (v: number) => peaks.push(v), exponentialRampToValueAtTime() {} }, connect: (n: unknown) => n };
    }
  }
  vi.stubGlobal('AudioContext', Ctx);
  return made;
}

// Node's process, without the Node types in this project.
const proc = (globalThis as unknown as { process: { on(e: string, f: () => void): void; off(e: string, f: () => void): void } }).process;

afterEach(() => {
  vi.runOnlyPendingTimers(); // close the context of this test
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('resumes_a_suspended_context_and_plays_two_tones', () => {
  vi.useFakeTimers();
  const made = fakeAudio('suspended');
  chime('join', null);
  expect(made).toHaveLength(1);
  expect(made[0]).toMatchObject({ resumed: 1, tones: 2 });
});

it('closes_after_10_s_idle_and_a_failed_close_is_caught', async () => {
  vi.useFakeTimers();
  const made = fakeAudio('running', true);
  const unhandled = vi.fn();
  proc.on('unhandledRejection', unhandled);
  chime('leave', null);
  expect(made[0].resumed).toBe(0);
  vi.advanceTimersByTime(9999);
  expect(made[0].closed).toBe(0);
  vi.advanceTimersByTime(1);
  expect(made[0].closed).toBe(1);
  vi.useRealTimers();
  await new Promise((r) => setTimeout(r, 10)); // the rejection would be reported by now
  proc.off('unhandledRejection', unhandled);
  expect(unhandled).not.toHaveBeenCalled();
  vi.useFakeTimers();
  chime('join', null); // a new context after the close
  expect(made).toHaveLength(2);
});

it('volume_0_opens_no_context', () => {
  vi.useFakeTimers();
  const made = fakeAudio('running');
  chime('mute', null, 0);
  expect(made).toHaveLength(0);
});

it('a_context_closed_elsewhere_is_replaced', () => {
  vi.useFakeTimers();
  const made = fakeAudio('running');
  chime('mute', null);
  (globalThis as unknown as { lastCtx: { state: string } }).lastCtx.state = 'closed';
  chime('unmute', null);
  expect(made).toHaveLength(2);
  expect(made[1].tones).toBe(2);
});

it.each(Object.keys(TONES) as Cue[])('%s_plays_its_pitches_lengths_and_gain', (cue) => {
  vi.useFakeTimers();
  fakeAudio('running');
  chime(cue, 'spk-1', 100);
  const ctx = (globalThis as unknown as { lastCtx: { oscs: { type?: string; frequency: { value?: number }; start?: number; stop?: number }[]; peaks: number[]; sinks: string[] } }).lastCtx;
  const t = TONES[cue];
  expect(ctx.oscs.map((o) => o.frequency.value)).toEqual(t.hz);
  expect(ctx.oscs.every((o) => o.type === t.wave)).toBe(true);
  for (const o of ctx.oscs) expect(o.stop! - o.start!).toBeCloseTo(t.step);
  expect(ctx.peaks).toEqual(t.hz.map(() => MAX_GAIN)); // volume 100 is the full peak
  expect(ctx.sinks).toEqual(['spk-1']);
});

it('the_default_volume_is_half_the_peak_and_the_default_speaker_needs_no_switch', () => {
  vi.useFakeTimers();
  fakeAudio('running');
  chime('mute', null);
  const ctx = (globalThis as unknown as { lastCtx: { peaks: number[]; sinks: string[] } }).lastCtx;
  expect(ctx.peaks).toEqual([0.08, 0.08]);
  expect(ctx.sinks).toEqual([]); // a new context already plays on the default speaker
});
