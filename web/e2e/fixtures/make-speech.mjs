// Writes speech.wav: 4 s, 48 kHz, mono, 16-bit. Synthetic, so it carries no third-party licence.
// Voiced syllables (a gliding pitch with three vowel formants) and short pauses, close enough to speech
// that browser noise suppression keeps it. Run: node e2e/fixtures/make-speech.mjs
import { writeFileSync } from 'node:fs';

const RATE = 48000;
const N = RATE * 4;
const pcm = new Int16Array(N);
// Formant centres in Hz for a few vowels, one per syllable in turn.
const vowels = [[700, 1200, 2600], [300, 2300, 3000], [500, 900, 2400], [400, 1900, 2600]];
const gain = (f, formants) => formants.reduce((g, c) => g + Math.exp(-(((f - c) / 120) ** 2)), 0);

let phase = 0; // pitch phase in cycles, summed so the glide stays at f0
for (let i = 0; i < N; i++) {
  const t = i / RATE;
  const f0 = 120 + 30 * Math.sin(2 * Math.PI * 0.7 * t);
  phase += f0 / RATE;
  const syl = Math.floor(t * 4); // four syllables a second
  const pos = t * 4 - syl;
  if (syl % 5 === 4 || pos > 0.8) continue; // a pause every fifth syllable and a gap between syllables
  const env = Math.sin((Math.PI * pos) / 0.8) ** 2;
  const v = vowels[syl % vowels.length];
  let s = 0;
  for (let h = 1; h * f0 < 4000; h++) s += (gain(h * f0, v) / h) * Math.sin(2 * Math.PI * h * phase);
  pcm[i] = Math.max(-32767, Math.min(32767, Math.round(s * env * 22000)));
}

const head = Buffer.alloc(44);
head.write('RIFF', 0);
head.writeUInt32LE(36 + N * 2, 4);
head.write('WAVEfmt ', 8);
head.writeUInt32LE(16, 16);
head.writeUInt16LE(1, 20); // PCM
head.writeUInt16LE(1, 22); // mono
head.writeUInt32LE(RATE, 24);
head.writeUInt32LE(RATE * 2, 28);
head.writeUInt16LE(2, 32);
head.writeUInt16LE(16, 34);
head.write('data', 36);
head.writeUInt32LE(N * 2, 40);
writeFileSync(new URL('speech.wav', import.meta.url), Buffer.concat([head, Buffer.from(pcm.buffer)]));
