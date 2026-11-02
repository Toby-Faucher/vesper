import { describe, expect, it, vi } from 'vitest';
import { DenoiseCore } from './denoise-core';

const Q = 128;
const blk = (v: number) => new Float32Array(Q).fill(v);
const frame = (v: number) => ({ type: 'frame' as const, data: new Float32Array(480).fill(v), ms: 1 });

function make() {
  const sent: Float32Array[] = [];
  const onFail = vi.fn();
  const core = new DenoiseCore(onFail, 48000);
  return { core, sent, onFail, connect: () => core.connect((f) => sent.push(f)) };
}
const render = (core: DenoiseCore, input?: Float32Array) => {
  const out = new Float32Array(Q).fill(NaN);
  core.render(input, out);
  return out;
};

describe('DenoiseCore', () => {
  // Hold the queue at `frames` entries while time passes; returns total dropped after each call.
  const hold = (t: ReturnType<typeof make>, frames: number, sec: number) => {
    for (let i = 0; i < (sec * 48000) / Q; i++) {
      while (t.core.queued < (frames - 0.5) * 480) t.core.reply(frame(1));
      render(t.core);
    }
    return t.core.stats().droppedOut;
  };

  it('trims a queue held above 5 frames for 2 s: one frame per second', () => {
    const t = make();
    t.connect();
    expect(hold(t, 6, 1.9)).toBe(0);
    expect(hold(t, 6, 0.2)).toBe(1);
    expect(hold(t, 6, 1)).toBe(2);
  });

  it('does not trim a queue at 5 frames', () => {
    const t = make();
    t.connect();
    expect(hold(t, 5, 6)).toBe(0);
  });

  it('passes the raw mic through before the worker is connected and after failure', () => {
    const t = make();
    expect(Array.from(render(t.core, blk(0.5)))).toEqual(Array.from(blk(0.5)));
    expect(Array.from(render(t.core))).toEqual(Array.from(blk(0))); // no input yet
    t.connect();
    t.core.fail('x');
    expect(render(t.core, blk(0.25))[0]).toBe(0.25);
    expect(t.sent).toHaveLength(0);
  });

  it('sends 480-sample frames and caps frames in flight at 4', () => {
    const t = make();
    t.connect();
    for (let i = 0; i < 20; i++) render(t.core, blk(i)); // 5 frames of input
    expect(t.sent).toHaveLength(4);
    expect(t.sent[0]).toHaveLength(480);
    expect(t.core.inflight).toBe(4);
    t.core.reply(frame(0));
    render(t.core, blk(0));
    expect(t.sent).toHaveLength(5);
  });

  it('stalls after half a second of in-flight frames with no reply', () => {
    const t = make();
    t.connect();
    let k = 0;
    while (t.sent.length === 0) render(t.core, blk(0));
    while (!t.core.failure && k < 1000) { k++; render(t.core, blk(0)); }
    // idle counts from the quantum that sent the first frame: 128 * (k + 1) > 24000
    expect(k).toBe(187);
    expect(t.core.failure).toBe('denoiser stalled');
    expect(t.onFail).toHaveBeenCalledTimes(1);
    expect(t.onFail).toHaveBeenCalledWith('denoiser stalled');
  });

  it('a startup burst of slow frames is ignored during warm-up', () => {
    const t = make();
    t.connect();
    for (let i = 0; i < 100; i++) t.core.reply({ ...frame(0), ms: 80 });
    expect(t.core.failure).toBeNull();
    expect(t.onFail).not.toHaveBeenCalled();
  });

  it('400 consecutive slow frames (300 warm-up + 100) trip the mode controller', () => {
    const t = make();
    t.connect();
    for (let i = 0; i < 319; i++) t.core.reply({ ...frame(0), ms: 80 }); // 300 warm-up + 19 late
    expect(t.core.failure).toBeNull();
    for (let i = 319; i < 400; i++) t.core.reply({ ...frame(0), ms: 80 });
    expect(t.onFail).toHaveBeenCalledTimes(1);
    expect(t.onFail).toHaveBeenCalledWith('denoiser too slow for real time');
  });

  it('worker error replies fail the core', () => {
    const t = make();
    t.connect();
    t.core.reply({ type: 'error', message: 'panic' });
    expect(t.onFail).toHaveBeenCalledWith('denoiser error: panic');
  });

  it('primes with three frames, re-primes after underrun', () => {
    const t = make();
    t.connect();
    t.core.reply(frame(1));
    t.core.reply(frame(2));
    expect(render(t.core)[0]).toBe(0);
    t.core.reply(frame(3));
    expect(t.core.queued).toBe(1440);
    const outs = Array.from({ length: 12 }, () => render(t.core));
    expect(outs[0][0]).toBe(0); // fades in from the silence before
    expect(outs[0][120]).toBe(1); // after the 2.5 ms fade
    expect(outs[4][0]).toBe(2); // samples 512.. come from frame 2
    expect(outs[8][0]).toBe(3); // samples 1024.. come from frame 3
    expect(outs[10][127]).toBe(3);
    expect(outs[11][0]).toBeCloseTo((3 * 31) / 32, 6); // the last 32 samples fade out
    expect(outs[11][31]).toBe(0);
    expect(outs[11][32]).toBe(0); // 1440 samples exhausted mid-quantum
    t.core.reply(frame(4));
    t.core.reply(frame(5));
    expect(render(t.core)[0]).toBe(0); // re-priming
  });

  it('caps the output queue at 8 frames, dropping the oldest', () => {
    const t = make();
    t.connect();
    for (let v = 1; v <= 10; v++) t.core.reply(frame(v));
    expect(t.core.queued).toBe(3840);
    expect(render(t.core)[127]).toBe(3);
    expect(t.core.queued).toBe(3840 - Q);
  });
});

describe('DenoiseCore underrun fade', () => {
  // A 440 Hz sine at 0.5: its own largest step is 2 pi 440 / 48000 * 0.5, about 0.029.
  const sine = (k: number) => {
    const f = new Float32Array(480);
    for (let i = 0; i < 480; i++) f[i] = 0.5 * Math.sin((2 * Math.PI * 440 * (k * 480 + i)) / 48000);
    return { type: 'frame' as const, data: f, ms: 1 };
  };
  const maxStep = (blocks: Float32Array[]) => {
    let prev = 0;
    let max = 0;
    for (const b of blocks) for (const x of b) [max, prev] = [Math.max(max, Math.abs(x - prev)), x];
    return max;
  };
  // Forced underruns: the worker stops replying, three times. 3 + extra frames run dry at sample 480 * (3 + extra) of the
  // round, so 1, 2 and 4 extra frames cut at three different points in a 128-sample block (32, 96 and 96 + 64).
  function run(extras: number[]) {
    const t = make();
    t.connect();
    let k = 0;
    const outs: Float32Array[] = [];
    for (const extra of extras) {
      for (let j = 0; j < 3; j++) t.core.reply(sine(k++));
      for (let b = 0; b < 4 * (3 + extra) + 6; b++) {
        outs.push(render(t.core));
        if (b % 3 === 0 && b / 3 < extra) t.core.reply(sine(k++)); // keeps playing for a while, then starves
      }
    }
    return { outs, stats: t.core.stats() };
  }

  it('an underrun is a fade, not a click (max sample step)', () => {
    const { outs, stats } = run([1, 2, 4]);
    expect(stats.underruns).toBe(3);
    expect(maxStep(outs)).toBeLessThan(0.035); // without the fade the step at the cut is up to 0.5
  });

  it('a queue that runs dry exactly at a block end fades out too, and fades back in', () => {
    const t = make();
    t.connect();
    for (let j = 0; j < 4; j++) t.core.reply(sine(j)); // 1920 samples = 15 blocks exactly
    const outs = Array.from({ length: 15 }, () => render(t.core));
    expect(Math.abs(outs[14][127])).toBe(0);
    t.core.reply(sine(4)); // arrives in time: no underrun, it fades in
    outs.push(render(t.core));
    expect(t.core.stats().underruns).toBe(0);
    expect(maxStep(outs)).toBeLessThan(0.035);
  });

  it('a steady queue is not touched after the first fade-in', () => {
    const t = make();
    t.connect();
    for (let j = 0; j < 8; j++) t.core.reply(frame(0.25));
    render(t.core);
    expect(Array.from(render(t.core))).toEqual(Array.from(blk(0.25)));
  });
});

describe('DenoiseCore stats', () => {
  const HALF_S = 188; // quanta in 0.5 s at 48 kHz: 188 * 128 >= 24000

  it('counts real and empty input quanta, also before connect', () => {
    const t = make();
    render(t.core, blk(0.1));
    render(t.core);
    render(t.core);
    expect(t.core.stats()).toMatchObject({ inputBlocks: 1, emptyInputBlocks: 2, state: 'passthrough', failure: null });
  });

  it('reports state through connect, priming, playing and failure', () => {
    const t = make();
    t.connect();
    expect(t.core.stats().state).toBe('connecting');
    t.core.reply(frame(1));
    expect(t.core.stats().state).toBe('priming');
    t.core.reply(frame(1));
    t.core.reply(frame(1));
    render(t.core);
    expect(t.core.stats().state).toBe('playing');
    expect(t.core.stats().queuedMs).toBeCloseTo((1440 - Q) / 48, 9);
    t.core.fail('boom');
    expect(t.core.stats()).toMatchObject({ state: 'failed', failure: 'boom' });
  });

  it('counts frames sent, replies and frames in flight', () => {
    const t = make();
    t.connect();
    for (let i = 0; i < 20; i++) render(t.core, blk(0));
    t.core.reply(frame(0));
    expect(t.core.stats()).toMatchObject({ framesSent: 4, replies: 1, inflight: 3 });
  });

  it('counts an underrun when the queue runs dry while playing, and a reprime when it restarts', () => {
    const t = make();
    t.connect();
    for (let v = 0; v < 3; v++) t.core.reply(frame(1));
    for (let i = 0; i < 11; i++) render(t.core);
    expect(t.core.stats().underruns).toBe(0);
    render(t.core); // only 32 samples left: a partial block
    render(t.core); // not playing any more: priming, not another underrun
    expect(t.core.stats()).toMatchObject({ underruns: 1, reprimes: 0, state: 'priming' });
    for (let v = 0; v < 3; v++) t.core.reply(frame(1));
    render(t.core);
    expect(t.core.stats()).toMatchObject({ underruns: 1, reprimes: 1, state: 'playing' });
  });

  it('counts dropped output frames and dropped input frames', () => {
    const t = make();
    t.connect();
    for (let v = 0; v < 10; v++) t.core.reply(frame(v));
    expect(t.core.stats().droppedOut).toBe(2);
    // 4 frames go to the worker and never come back; the 4 frame ring then overflows.
    for (let i = 0; i < 40; i++) render(t.core, blk(0)); // 5120 samples = 1920 sent + 1920 kept + 1280 dropped
    expect(t.core.stats().droppedInput).toBe(3); // 1280 samples, rounded up to frames
  });

  it('reports last, average and max denoiser time over the last 100 replies', () => {
    const t = make();
    t.connect();
    expect(t.core.stats()).toMatchObject({ lastReplyMs: 0, avgReplyMs: 0, maxReplyMs: 0 });
    for (const ms of [1, 2, 6]) t.core.reply({ ...frame(0), ms });
    expect(t.core.stats()).toMatchObject({ lastReplyMs: 6, avgReplyMs: 3, maxReplyMs: 6 });
    for (let i = 0; i < 100; i++) t.core.reply({ ...frame(0), ms: 2 });
    expect(t.core.stats()).toMatchObject({ lastReplyMs: 2, avgReplyMs: 2, maxReplyMs: 2 });
  });

  it('input and output RMS cover the last half second; missing input counts as silence', () => {
    const t = make();
    for (let i = 0; i < HALF_S; i++) render(t.core, blk(0.5));
    expect(t.core.stats().inputRms).toBeCloseTo(0.5, 6);
    expect(t.core.stats().outputRms).toBeCloseTo(0.5, 6); // passthrough
    for (let i = 0; i < HALF_S; i++) render(t.core);
    expect(t.core.stats().inputRms).toBe(0);
    expect(t.core.stats().outputRms).toBe(0);
    for (let i = 0; i < HALF_S; i++) render(t.core, i % 2 ? blk(0.45) : blk(-0.45)); // under the limiter knee
    expect(t.core.stats().inputRms).toBeCloseTo(0.45, 6);
  });

  it('inputPeak is the boosted input before the limiter, over the last half second', () => {
    const t = make();
    t.core.limiter.set(true);
    let top = 0;
    for (let i = 0; i < HALF_S; i++) top = Math.max(top, ...render(t.core, blk(i === 5 ? 2 : 0.1)));
    expect(t.core.stats().inputPeak).toBe(2);
    expect(top).toBeLessThan(1); // what passed on (the passthrough here) was limited
    for (let i = 0; i < HALF_S; i++) render(t.core, blk(0.1));
    expect(t.core.stats().inputPeak).toBeCloseTo(0.1, 6);
  });

  it('stats are plain numbers and strings', () => {
    const s = make().core.stats();
    expect(JSON.parse(JSON.stringify(s))).toEqual(s);
  });
});
