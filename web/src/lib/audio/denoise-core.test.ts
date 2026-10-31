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
  it('passes the raw mic through before the worker is connected and after failure', () => {
    const t = make();
    expect(Array.from(render(t.core, blk(0.5)))).toEqual(Array.from(blk(0.5)));
    expect(Array.from(render(t.core))).toEqual(Array.from(blk(0))); // no input yet
    t.connect();
    t.core.fail('x');
    expect(render(t.core, blk(0.25))[0]).toBe(0.25);
    expect(t.sent).toHaveLength(0);
  });

  it('sends 480-sample frames and caps frames in flight at 2', () => {
    const t = make();
    t.connect();
    for (let i = 0; i < 12; i++) render(t.core, blk(i)); // 3 frames of input
    expect(t.sent).toHaveLength(2);
    expect(t.sent[0]).toHaveLength(480);
    expect(t.core.inflight).toBe(2);
    t.core.reply(frame(0));
    render(t.core, blk(0));
    expect(t.sent).toHaveLength(3);
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

  it('slow frames trip the mode controller', () => {
    const t = make();
    t.connect();
    for (let i = 0; i < 9; i++) t.core.reply({ ...frame(0), ms: 15 });
    expect(t.core.failure).toBeNull();
    t.core.reply({ ...frame(0), ms: 15 });
    expect(t.core.failure).toBe('denoiser too slow for real time');
  });

  it('worker error replies fail the core', () => {
    const t = make();
    t.connect();
    t.core.reply({ type: 'error', message: 'panic' });
    expect(t.onFail).toHaveBeenCalledWith('denoiser error: panic');
  });

  it('primes with two frames, re-primes after underrun', () => {
    const t = make();
    t.connect();
    t.core.reply(frame(1));
    expect(render(t.core)[0]).toBe(0);
    t.core.reply(frame(2));
    expect(t.core.queued).toBe(960);
    const outs = Array.from({ length: 8 }, () => render(t.core));
    expect(outs[0][0]).toBe(1);
    expect(outs[4][0]).toBe(2); // samples 512.. come from frame 2
    expect(outs[7][63]).toBe(2);
    expect(outs[7][64]).toBe(0); // 960 samples exhausted mid-quantum
    t.core.reply(frame(3));
    expect(render(t.core)[0]).toBe(0); // re-priming
  });

  it('caps the output queue at 3 frames, dropping the oldest', () => {
    const t = make();
    t.connect();
    for (let v = 1; v <= 5; v++) t.core.reply(frame(v));
    expect(t.core.queued).toBe(1440);
    expect(render(t.core)[0]).toBe(3);
    expect(t.core.queued).toBe(1440 - Q);
  });
});
