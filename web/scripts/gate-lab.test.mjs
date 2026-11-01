// @vitest-environment node
// The noise gate on real denoised speech. Opt in, it needs the audio lab corpus (docs/research-loop.md):
//   target/release/audiolab listen --config empty.toml --items dev-0016,... --out $DIR --corpus research/corpus48/built
//   VESPER_LAB_DIR=$DIR VESPER_CORPUS_DIR=research/corpus48/built/dev npx vitest run scripts/gate-lab.test.mjs
// The processed WAVs are the shipped DeepFilterNet output; they replay through DenoiseCore as the worker's replies,
// so the gate sees what it sees in the browser. The marker is set the way a user would: 6 dB over the residual
// noise peak in the 1.5 s lead-in. Speech and pauses come from the clean reference.
import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { DenoiseCore } from '../src/lib/audio/denoise-core';
import { LIMIT_CEILING, Limiter } from '../src/lib/audio/dynamics';

const LAB = process.env.VESPER_LAB_DIR;
const CORPUS = process.env.VESPER_CORPUS_DIR;
const SR = 48000;
const MS = 48;
const Q = 128;
const db = (r) => 20 * Math.log10(r);

function wav(path) {
  const b = readFileSync(path);
  let bits = 16;
  for (let i = 12; i < b.length; ) {
    const id = b.toString('ascii', i, i + 4);
    const n = b.readUInt32LE(i + 4);
    if (id === 'fmt ') bits = b.readUInt16LE(i + 22);
    if (id === 'data') {
      const d = b.subarray(i + 8, i + 8 + n);
      return bits === 32 ? new Float32Array(d.buffer.slice(d.byteOffset, d.byteOffset + n))
        : Float32Array.from(new Int16Array(d.buffer.slice(d.byteOffset, d.byteOffset + n)), (v) => v / 32768);
    }
    i += 8 + n + (n & 1);
  }
  throw new Error(`no data in ${path}`);
}

// Replies arrive 1 to 8 quanta after the frame is sent, like a jittery worker; the reply is the denoised frame.
function run(den, gate) {
  const core = new DenoiseCore(() => {}, SR);
  let s = 5;
  const r = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32);
  const pending = [];
  let q = 0;
  let sent = 0;
  core.connect(() => { pending.push({ at: q + 1 + Math.floor(r() * 8), data: den.slice(sent, sent + 480) }); sent += 480; });
  core.gate.set(gate.on, gate.thresholdDb);
  const y = new Float32Array(den.length);
  const zeros = new Float32Array(Q);
  for (; q * Q < den.length; q++) {
    pending.sort((a, b) => a.at - b.at);
    while (pending.length && pending[0].at <= q) core.reply({ type: 'frame', data: pending.shift().data, ms: 1 });
    core.render(zeros, y.subarray(q * Q, q * Q + Q));
  }
  expect(core.stats().underruns).toBe(0);
  return y;
}

const energy = (y, a, b) => { let e = 0; for (let i = a; i < b; i++) e += y[i] * y[i]; return e; };
const peak = (y, a, b) => { let p = 0; for (let i = a; i < b; i++) p = Math.max(p, Math.abs(y[i])); return p; };

describe.skipIf(!LAB || !CORPUS)('gate on real denoised speech (audio lab)', () => {
  const ids = LAB ? readdirSync(LAB).filter((f) => f.endsWith('.processed.wav')).map((f) => f.split('.')[0]).sort() : [];
  it.each(ids)('%s', (id) => {
    const den = wav(`${LAB}/${id}.processed.wav`);
    const clean = wav(`${CORPUS}/${id}.clean.wav`);
    const marker = Math.min(0, Math.round(db(peak(den, 0.2 * SR, 1.5 * SR))) + 6);
    const ref = run(den, { on: false, thresholdDb: marker });
    const out = run(den, { on: true, thresholdDb: marker });
    const w = 3 * SR; // the output queue delay: where the ungated output repeats the denoised input
    let L = 0;
    while (L < 5000 && !den.subarray(w, w + 256).every((v, i) => v === ref[w + L + i])) L++;
    expect(L).toBeLessThan(5000);
    // Speech and pauses from the clean reference: 10 ms frames whose peak is within 40 dB of the loudest frame.
    const frames = Math.floor(clean.length / 480) - 10;
    const top = peak(clean, 0, clean.length);
    const active = Array.from({ length: frames }, (_, f) => peak(clean, f * 480, f * 480 + 480) > top / 100);
    let sp = [0, 0];
    for (let f = 0; f < frames; f++) {
      if (active[f]) sp = [sp[0] + energy(out, f * 480 + L, f * 480 + 480 + L), sp[1] + energy(ref, f * 480 + L, f * 480 + 480 + L)];
    }
    const speech = db(Math.sqrt(sp[0] / sp[1]));
    // Word onsets as the gate must see them: the first sample over the marker after 300 ms under it. The 20 ms from
    // there must pass unchanged, and the 20 ms before it (the quiet start the gate ramps in on) is reported.
    const thr = 10 ** (marker / 20);
    let worstOnset = 0;
    let worstPre = 0;
    let onsets = 0;
    for (let i = 0, quiet = 0; i < ref.length - 20 * MS; i++) {
      if (Math.abs(ref[i]) <= thr) { quiet++; continue; }
      if (quiet >= 300 * MS && i > 1.5 * SR + L) {
        onsets++;
        worstOnset = Math.min(worstOnset, db(Math.sqrt(energy(out, i, i + 20 * MS) / energy(ref, i, i + 20 * MS))));
        const pre = energy(ref, i - 20 * MS, i);
        if (pre > 0) worstPre = Math.min(worstPre, db(Math.sqrt((energy(out, i - 20 * MS, i) + energy(out, i, i + 20 * MS)) / (pre + energy(ref, i, i + 20 * MS)))));
      }
      quiet = 0;
    }
    const lead = db(Math.sqrt(energy(out, 0.4 * SR + L, 1.4 * SR + L) / energy(ref, 0.4 * SR + L, 1.4 * SR + L)));
    let stepOut = 0;
    let stepRef = 0;
    for (let i = 1; i < out.length - L; i++) {
      if (active[Math.floor((i - L) / 480)] || active[Math.floor((i - L) / 480) + 1]) continue;
      stepOut = Math.max(stepOut, Math.abs(out[i] - out[i - 1]));
      stepRef = Math.max(stepRef, Math.abs(ref[i] - ref[i - 1]));
    }
    console.log(`${id}: marker ${marker} dB, lag ${(L / MS).toFixed(1)} ms, speech ${speech.toFixed(2)} dB, ${onsets} onsets: first 20 ms ${worstOnset.toFixed(2)} dB, with the 20 ms before ${worstPre.toFixed(2)} dB, lead-in noise ${lead.toFixed(1)} dB, max step outside speech ${stepOut.toExponential(1)} (ungated ${stepRef.toExponential(1)})`);
    expect(stepOut).toBeLessThanOrEqual(stepRef);
    expect(worstOnset).toBeGreaterThan(-0.5);
  });
});

// The limiter on real clean speech, scaled to peak at 0 dBFS (a hot mic) and boosted by 6 and 12 dB.
describe.skipIf(!LAB || !CORPUS)('limiter on real speech (audio lab)', () => {
  const ids = LAB ? readdirSync(LAB).filter((f) => f.endsWith('.processed.wav')).map((f) => f.split('.')[0]).sort() : [];
  it.each(ids)('%s', (id) => {
    const clean = wav(`${CORPUS}/${id}.clean.wav`);
    const p = peak(clean, 0, clean.length);
    const row = [6, 12].map((boost) => {
      const x = clean.map((v) => (v / p) * 10 ** (boost / 20));
      const y = new Float32Array(x.length);
      const l = new Limiter(SR);
      for (let i = 0; i < x.length; i += Q) l.process(x.subarray(i, i + Q), y.subarray(i, i + Q));
      const over = x.filter((v) => Math.abs(v) >= 1).length;
      expect(peak(y, 0, y.length)).toBeLessThanOrEqual(Math.fround(LIMIT_CEILING));
      // Waveform distortion: per 10 ms frame, what is left after the best single gain for that frame, so a slow
      // gain change (what a limiter should do) does not count and a flattened peak (what a clip does) does.
      const err = (z) => {
        let [e, t] = [0, 0];
        for (let a = 0; a + 480 <= z.length; a += 480) {
          let [zx, xx] = [0, 0];
          for (let i = a; i < a + 480; i++) [zx, xx] = [zx + z[i] * x[i], xx + x[i] * x[i]];
          const g = xx ? zx / xx : 0;
          for (let i = a; i < a + 480; i++) [e, t] = [e + (z[i] - g * x[i]) ** 2, t + z[i] * z[i]];
        }
        return db(Math.sqrt(e / t));
      };
      const clip = x.map((v) => Math.max(-1, Math.min(1, v)));
      expect(err(y)).toBeLessThan(err(clip));
      return `+${boost} dB: ${over} samples over full scale, limiter peak ${db(peak(y, 0, y.length)).toFixed(2)} dBFS, distortion per 10 ms frame: limiter ${err(y).toFixed(1)} dB, hard clip ${err(clip).toFixed(1)} dB`;
    });
    console.log(`${id}: ${row.join('; ')}`);
  });
});
