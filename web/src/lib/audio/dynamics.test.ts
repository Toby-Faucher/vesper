import { describe, expect, it } from 'vitest';
import { DenoiseCore } from './denoise-core';
import { Gate, LIMIT_CEILING, LIMIT_KNEE, Limiter, softCurve } from './dynamics';

// Synthetic signals only: these numbers are not real-voice verified. web/scripts/gate-lab.test.mjs repeats the checks on
// real denoised corpus speech when VESPER_LAB_DIR is set.
const SR = 48000;
const Q = 128;
const dbOf = (r: number) => 20 * Math.log10(r);

function rng(seed: number) {
  let s = seed >>> 0;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32) * 2 - 1;
}

/** Voiced syllables: 120 Hz harmonics with vowel formant weights under a 4 Hz syllable envelope, peak normalised to 1. */
function speechLike(seconds: number) {
  const n = seconds * SR;
  const x = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const t = i / SR;
    const f0 = 120 * (1 + 0.1 * Math.sin(2 * Math.PI * 0.7 * t));
    let v = 0;
    for (let k = 1; k <= 30; k++) {
      const f = k * f0;
      const w = Math.exp(-(((f - 700) / 200) ** 2)) + 0.6 * Math.exp(-(((f - 1200) / 250) ** 2)) + 0.3 * Math.exp(-(((f - 2600) / 400) ** 2)) + 0.05;
      v += (w / k) * Math.sin(2 * Math.PI * k * 120 * t + 0.6 * k);
    }
    x[i] = v * Math.max(0, Math.sin(2 * Math.PI * 2 * t)) ** 0.5;
  }
  const peak = x.reduce((m, v) => Math.max(m, Math.abs(v)), 0);
  return x.map((v) => v / peak);
}

function limit(x: Float32Array, on = true) {
  const l = new Limiter(SR);
  l.set(on);
  const y = new Float32Array(x.length);
  for (let i = 0; i < x.length; i += Q) l.process(x.subarray(i, i + Q), y.subarray(i, i + Q));
  return y;
}

/** THD of a steady sine over a whole number of periods: harmonics 2..20 against the fundamental. */
function thd(y: Float32Array, f: number, from: number, len: number) {
  const amp = (h: number) => {
    let re = 0;
    let im = 0;
    for (let i = from; i < from + len; i++) {
      re += y[i] * Math.cos((2 * Math.PI * h * f * i) / SR);
      im += y[i] * Math.sin((2 * Math.PI * h * f * i) / SR);
    }
    return Math.hypot(re, im);
  };
  let sum = 0;
  for (let h = 2; h <= 20; h++) sum += amp(h) ** 2;
  return Math.sqrt(sum) / amp(1);
}

describe('Limiter', () => {
  it('the curve is the identity up to the knee, continuous and below the ceiling above it', () => {
    expect(softCurve(LIMIT_KNEE)).toBe(LIMIT_KNEE);
    expect(softCurve(0.3)).toBe(0.3);
    expect(softCurve(LIMIT_KNEE + 1e-6)).toBeCloseTo(LIMIT_KNEE + 1e-6, 9);
    for (const x of [0.6, 1, 2, 4]) expect(softCurve(x)).toBeLessThan(LIMIT_CEILING);
    expect(softCurve(100)).toBeLessThanOrEqual(LIMIT_CEILING); // rounds to it in float
  });

  it('a level that falls slower than the release after the hold stays under the ceiling', () => {
    const x = new Float32Array(4000).fill(19.999); // far over the knee, where the curve is at the ceiling
    x[0] = 20;
    expect(limit(x).every((v) => Math.abs(v) <= Math.fround(LIMIT_CEILING))).toBe(true);
  });

  it('is bit-exact below the knee (-6 dBFS)', () => {
    const x = speechLike(1).map((v) => v * LIMIT_KNEE);
    expect(Array.from(limit(x))).toEqual(Array.from(x));
  });

  // A hot mic: the speech-like signal peaks at 0 dBFS before the boost. Without a limiter the float signal reaches the
  // encoder and is clipped at full scale there; that hard clip is the baseline.
  it.each([[6], [12]])('speech-like peaks at +%i dB boost stay under -1 dBFS; hard clip counts', (boostDb) => {
    const g = 10 ** (boostDb / 20);
    const x = speechLike(2).map((v) => v * g);
    const y = limit(x);
    const peak = y.reduce((m, v) => Math.max(m, Math.abs(v)), 0);
    const over = x.filter((v) => Math.abs(v) >= 1).length;
    console.log(`limiter +${boostDb} dB: input peak ${dbOf(g).toFixed(1)} dBFS, ${over} samples (${((100 * over) / x.length).toFixed(1)}%) a hard clip would cut; limiter peak ${dbOf(peak).toFixed(2)} dBFS, 0 clipped`);
    expect(peak).toBeLessThanOrEqual(Math.fround(LIMIT_CEILING)); // the output is float32
    expect(over).toBeGreaterThan(1000);
    expect(y.filter((v) => Math.abs(v) >= 1)).toHaveLength(0);
  });

  // 80 Hz: a low male pitch, the longest period the gain must hold through.
  it.each([[6], [12]])('an 80 Hz sine at +%i dBFS: far lower THD than a hard clip', (overDb) => {
    const a = 10 ** (overDb / 20);
    const x = new Float32Array(SR).map((_, i) => a * Math.sin((2 * Math.PI * 80 * i) / SR));
    const lim = thd(limit(x), 80, SR / 2, 4800);
    const clip = thd(x.map((v) => Math.max(-1, Math.min(1, v))), 80, SR / 2, 4800);
    console.log(`THD at +${overDb} dBFS: limiter ${(100 * lim).toFixed(3)}%, hard clip at 0 dBFS ${(100 * clip).toFixed(1)}%`);
    expect(lim).toBeLessThan(0.01);
    expect(clip).toBeGreaterThan(0.1);
  });
});

/**
 * DenoiseCore with an identity "denoiser": each sent frame comes back after `delay()` quanta. The real denoiser's
 * reply time does not change how the gate and the queue interact, which is what is measured here.
 */
function pipeline(x: Float32Array, gate: { on: boolean; thresholdDb: number }, delay: () => number) {
  const core = new DenoiseCore(() => {}, SR);
  const pending: { at: number; data: Float32Array }[] = [];
  let q = 0;
  core.connect((f) => pending.push({ at: q + delay(), data: f.slice() }));
  core.gate.set(gate.on, gate.thresholdDb);
  const y = new Float32Array(x.length);
  for (; q * Q < x.length; q++) {
    pending.sort((a, b) => a.at - b.at);
    while (pending.length && pending[0].at <= q) core.reply({ type: 'frame', data: pending.shift()!.data, ms: 1 });
    core.render(x.subarray(q * Q, q * Q + Q), y.subarray(q * Q, q * Q + Q));
  }
  expect(core.stats().underruns).toBe(0);
  return y;
}

/** Fan-like noise (low-passed, peak about -55 dBFS) with words (peak -20 dBFS) that start at full level in one sample. */
function scene() {
  const r = rng(7);
  const x = new Float32Array(6 * SR);
  let lp = 0;
  for (let i = 0; i < x.length; i++) x[i] = (lp += 0.05 * (r() - lp)) * 0.004;
  const noisePeak = x.reduce((m, v) => Math.max(m, Math.abs(v)), 0);
  const voice = speechLike(1);
  const words: [number, number][] = [];
  for (const [s, len] of [[0.5, 0.6], [1.6, 0.25], [2.3, 0.8], [3.9, 0.4], [4.9, 0.6]]) {
    const a = Math.round(s * SR);
    const b = a + Math.round(len * SR);
    // A plosive onset: a 2 ms click at full word level, then the voiced part with no fade in.
    for (let i = a; i < b; i++) x[i] += 0.1 * (i - a < 96 ? r() : voice[(i - a + SR / 8) % SR]);
    words.push([a, b]);
  }
  return { x, words, noisePeak };
}

const energy = (y: Float32Array, a: number, b: number) => y.subarray(a, b).reduce((s, v) => s + v * v, 0);
function lag(x: Float32Array, y: Float32Array) {
  for (let L = 0; L < 5000; L++) if (x.subarray(SR / 5, SR / 5 + 2000).every((v, i) => v === y[SR / 5 + L + i])) return L;
  throw new Error('no lag');
}

describe('Gate on the DenoiseCore output', () => {
  const MS = SR / 1000;
  const cases: [string, () => () => number][] = [
    ['steady replies (2 quanta)', () => () => 2],
    ['jittery replies (1 to 8 quanta)', () => { const r = rng(3); return () => 1 + Math.floor((r() + 1) * 4); }],
  ];
  it.each(cases)('%s: onsets kept, words unchanged, noise cut >= 20 dB, no clicks', (_, mk) => {
    const { x, words, noisePeak } = scene();
    expect(dbOf(noisePeak)).toBeLessThan(-54); // under the close level of a -50 dB marker
    const ref = pipeline(x, { on: false, thresholdDb: -50 }, mk());
    const out = pipeline(x, { on: true, thresholdDb: -50 }, mk());
    const L = lag(x, ref);
    const rows: string[] = [];
    let worstOnset = 0;
    let worstWord = 0;
    let leastCut = Infinity;
    words.forEach(([a, b], k) => {
      const onset = dbOf(Math.sqrt(energy(out, a + L, a + L + 20 * MS) / energy(ref, a + L, a + L + 20 * MS)));
      const first = dbOf(Math.sqrt(energy(out, a + L, a + L + 2 * MS) / energy(ref, a + L, a + L + 2 * MS)));
      const word = dbOf(Math.sqrt(energy(out, a + L, b + L) / energy(ref, a + L, b + L)));
      worstOnset = Math.min(worstOnset, onset, first);
      worstWord = Math.max(worstWord, Math.abs(word));
      const next = words[k + 1]?.[0] ?? x.length - L;
      // Noise from the end of hold plus release (320 ms) to 90 ms before the next word: the gate opens up to a full
      // output queue (80 ms) early, which is the lookahead.
      const [na, nb] = [b + L + 330 * MS, next + L - 90 * MS];
      if (nb - na > 50 * MS) {
        const cut = dbOf(Math.sqrt(energy(out, na, nb) / energy(ref, na, nb)));
        leastCut = Math.min(leastCut, -cut);
        rows.push(`word ${k}: first 2 ms ${first.toFixed(2)} dB, first 20 ms ${onset.toFixed(2)} dB, word ${word.toFixed(2)} dB, noise after ${cut.toFixed(1)} dB`);
      } else rows.push(`word ${k}: first 2 ms ${first.toFixed(2)} dB, first 20 ms ${onset.toFixed(2)} dB, word ${word.toFixed(2)} dB`);
    });
    const step = (y: Float32Array) => y.reduce((m, v, i) => (i ? Math.max(m, Math.abs(v - y[i - 1])) : 0), 0);
    // Steps in the noise only: in a word the gate is fully open, so any step there is the signal's own.
    const noiseStep = (y: Float32Array) => {
      let m = 0;
      for (let i = 1; i < y.length; i++) if (!words.some(([a, b]) => i >= a + L - 5 * MS && i < b + L)) m = Math.max(m, Math.abs(y[i] - y[i - 1]));
      return m;
    };
    console.log(`gate, ${_}: noise peak ${dbOf(noisePeak).toFixed(1)} dBFS, lag ${(L / MS).toFixed(1)} ms\n  ${rows.join('\n  ')}\n  max step: gated ${step(out).toExponential(2)}, ungated ${step(ref).toExponential(2)}; outside words gated ${noiseStep(out).toExponential(2)}, ungated ${noiseStep(ref).toExponential(2)}`);
    expect(worstOnset).toBeGreaterThan(-0.5);
    expect(worstWord).toBeLessThan(0.5);
    expect(leastCut).toBeGreaterThanOrEqual(20);
    expect(step(out)).toBeLessThanOrEqual(step(ref));
    expect(noiseStep(out)).toBeLessThanOrEqual(noiseStep(ref));
  });

  it('a pause shorter than the hold inside speech is not gated', () => {
    const x = new Float32Array(2 * SR).fill(0);
    for (let i = 0; i < x.length; i++) if (i < 0.8 * SR || i > 0.95 * SR) x[i] = 0.1 * Math.sin(i / 10); // 150 ms pause
    const ref = pipeline(x, { on: false, thresholdDb: -50 }, () => 2);
    const out = pipeline(x, { on: true, thresholdDb: -50 }, () => 2);
    expect(Array.from(out)).toEqual(Array.from(ref));
  });

  it('above the marker the gate never closes; it closes after the hold once below marker minus 4 dB', () => {
    const lvl = (db: number) => 10 ** (db / 20);
    // 1 s at -52 dBFS (between close and open: an open gate stays open), then silence.
    const x = new Float32Array(2 * SR);
    for (let i = 0; i < SR; i++) x[i] = lvl(-52) * (i % 2 ? 1 : -1);
    x[100] = lvl(-40); // opens it at the start
    const out = pipeline(x, { on: true, thresholdDb: -50 }, () => 2);
    const ref = pipeline(x, { on: false, thresholdDb: -50 }, () => 2);
    expect(Array.from(out)).toEqual(Array.from(ref)); // after the signal ends, both are silent
    // A threshold above the signal: closes after hold (200 ms) and release (120 ms), well inside the 1 s.
    const shut = pipeline(x, { on: true, thresholdDb: -30 }, () => 2);
    expect(energy(shut, 0.5 * SR, SR)).toBe(0);
  });
});

const maxStep = (y: Float32Array) => y.reduce((m, v, i) => (i ? Math.max(m, Math.abs(v - y[i - 1])) : 0), 0);

describe('Limiter off (no boost, the default)', () => {
  // A hot mic at 0 dB boost: what the user hears today must not change at all.
  it.each([[-3], [0]])('speech-like peaks at %i dBFS pass bit-identical, alone and through DenoiseCore', (peakDb) => {
    const x = speechLike(1).map((v) => v * 10 ** (peakDb / 20));
    expect(Array.from(limit(x, false))).toEqual(Array.from(x));
    const core = new DenoiseCore(() => {}, SR); // never told about a boost: off
    const y = new Float32Array(x.length);
    for (let i = 0; i < x.length; i += Q) core.render(x.subarray(i, i + Q), y.subarray(i, i + Q)); // passthrough
    expect(Array.from(y)).toEqual(Array.from(x));
    expect(core.stats().inputPeak).toBeGreaterThan(0); // the meter still sees it
  });

  it('turning the boost on and off live crossfades in 10 ms: no step, limited once on, bit-exact once off', () => {
    const x = new Float32Array(SR).map((_, i) => 2 * Math.sin((2 * Math.PI * 200 * i) / SR));
    const l = new Limiter(SR);
    const y = new Float32Array(x.length);
    for (let i = 0; i < x.length; i += Q) {
      if (i === 0.3 * SR - ((0.3 * SR) % Q)) l.set(true);
      if (i === 0.7 * SR - ((0.7 * SR) % Q)) l.set(false);
      l.process(x.subarray(i, i + Q), y.subarray(i, i + Q));
    }
    console.log(`limiter switch: max step ${maxStep(y).toFixed(4)}, the sine's own ${maxStep(x).toFixed(4)}`);
    expect(maxStep(y)).toBeLessThanOrEqual(maxStep(x));
    expect(y.subarray(0.35 * SR, 0.65 * SR).every((v) => Math.abs(v) <= Math.fround(LIMIT_CEILING))).toBe(true);
    expect(Array.from(y.subarray(0.75 * SR))).toEqual(Array.from(x.subarray(0.75 * SR)));
  });
});

describe('Bad samples', () => {
  it('a NaN or Infinity sample becomes 0 and does not latch the limiter', () => {
    const x = new Float32Array(SR).map((_, i) => 0.3 * Math.sin(i / 9));
    x[1000] = NaN;
    x[30000] = Infinity;
    x[30001] = -Infinity;
    const y = limit(x);
    expect([y[1000], y[30000], y[30001]]).toEqual([0, 0, 0]);
    const good = (i: number) => i !== 1000 && i !== 30000 && i !== 30001;
    expect(Array.from(y).filter((_, i) => good(i))).toEqual(Array.from(x).filter((_, i) => good(i)));
  });

  it('a NaN frame does not latch the gate', () => {
    const g = new Gate(SR);
    g.set(true, -50);
    for (let k = 0; k < 40; k++) g.detect(new Float32Array(480)); // 400 ms quiet: closed
    g.detect(new Float32Array(480).fill(NaN));
    g.detect(new Float32Array(480).fill(0.1)); // loud: opens
    const o = new Float32Array(960).fill(0.1);
    g.apply(o);
    expect(o[959]).toBeCloseTo(0.1, 6);
  });
});

describe('Gate state', () => {
  const closed = (thresholdDb = -1) => {
    const g = new Gate(SR);
    g.set(true, thresholdDb);
    for (let k = 0; k < 40; k++) {
      g.detect(new Float32Array(480).fill(0.5)); // -6 dBFS: under -1 minus 4 dB, so quiet
      g.apply(new Float32Array(480).fill(0.5));
    }
    return g;
  };

  it('a marker move while closed keeps it closed', () => {
    const g = closed();
    const o = new Float32Array(480).fill(0.5);
    g.apply(o);
    expect(o.every((v) => v === 0)).toBe(true);
    g.set(true, -2); // still above the signal
    g.detect(new Float32Array(480).fill(0.5));
    g.apply(o.fill(0.5));
    expect(o.every((v) => v === 0)).toBe(true);
  });

  it('turning it off while closed ramps open in 2 ms, without a step', () => {
    const g = closed();
    g.set(false, -1);
    const o = new Float32Array(480).fill(0.5);
    g.apply(o);
    expect(o[0]).toBeLessThan(0.01);
    expect(maxStep(o)).toBeLessThan(0.01);
    expect(o[479]).toBe(0.5);
  });

  it('turned on at startup, before the first reply: a word at once is kept, the silence after it closes', () => {
    const x = new Float32Array(SR);
    for (let i = 0; i < 0.3 * SR; i++) x[i] = 0.1 * Math.sin(i / 7);
    const ref = pipeline(x, { on: false, thresholdDb: -50 }, () => 2);
    const out = pipeline(x, { on: true, thresholdDb: -50 }, () => 2);
    expect(Array.from(out)).toEqual(Array.from(ref)); // the tail is digital silence in both
  });

  it('near an underrun (under 2 ms queued when the word arrives) the first 20 ms lose under 0.5 dB', () => {
    const core = new DenoiseCore(() => {}, SR);
    core.connect(() => {});
    core.gate.set(true, -50);
    const o = new Float32Array(Q);
    for (let q = 0; q < 300; q++) { // ~0.8 s of quiet, playing: the gate closes
      while (core.queued < 1000) core.reply({ type: 'frame', data: new Float32Array(480).fill(1e-4), ms: 1 });
      core.render(undefined, o);
    }
    while (core.queued > Q) core.render(undefined, o); // drain to under one quantum, still playing
    const left = core.queued;
    const word = new Float32Array(960).map((_, i) => 0.1 * Math.sin(i / 7));
    core.reply({ type: 'frame', data: word.slice(0, 480), ms: 1 });
    core.reply({ type: 'frame', data: word.slice(480), ms: 1 });
    const y = new Float32Array(Math.ceil((left + 960) / Q) * Q);
    for (let i = 0; i < y.length; i += Q) core.render(undefined, y.subarray(i, i + Q));
    const got = y.subarray(left, left + 960);
    const loss = dbOf(Math.sqrt(energy(got, 0, 960) / energy(word, 0, 960)));
    const first2 = dbOf(Math.sqrt(energy(got, 0, 96) / energy(word, 0, 96)));
    console.log(`near underrun: ${left} samples queued ahead, first 2 ms ${first2.toFixed(2)} dB, first 20 ms ${loss.toFixed(2)} dB`);
    expect(left).toBeLessThan(96);
    expect(loss).toBeGreaterThan(-0.5);
  });
});
