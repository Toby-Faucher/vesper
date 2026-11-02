// The limiter and the noise gate of the deepfilter capture path. Both run inside DenoiseCore on the audio thread,
// so they allocate nothing per call. Levels are sample peaks in dBFS, the same scale as the Settings meter.

const db = (v: number) => 10 ** (v / 20);

/** Limiter: below the knee the gain is exactly 1; above it a soft curve that never reaches the ceiling. */
export const LIMIT_KNEE = db(-6);
export const LIMIT_CEILING = db(-1);
const LIMIT_HOLD_S = 0.02; // longer than a voice pitch period (50 Hz and up), so the gain does not ripple within one
const LIMIT_RELEASE_S = 0.08;

/** The static curve: identity up to the knee, then an exponential approach to the ceiling (slope 1 at the knee). */
export function softCurve(x: number): number {
  if (x <= LIMIT_KNEE) return x;
  const w = LIMIT_CEILING - LIMIT_KNEE;
  return LIMIT_CEILING - w * Math.exp(-(x - LIMIT_KNEE) / w);
}

const LIMIT_FADE_S = 0.01; // crossfade between the dry and the limited signal when the boost turns on or off
const LIMIT_SANE = 1e3; // a sample beyond this (or NaN) is a fault, not audio: it becomes 0 and leaves the state alone

/**
 * A peak limiter for the boosted mic, before the denoiser. The envelope jumps to each new peak at once and holds it,
 * then decays, and the gain is softCurve(env) / env. Since env >= |x|, every output sample stays below the ceiling
 * without a lookahead delay; between peaks the gain is smooth, so the waveform is scaled, not clipped.
 * Off (no boost, the default) the output is the input, bit for bit: at 0 dB nothing can pass full scale anyway.
 * Switching crossfades over 10 ms; the envelope always runs, so it is ready when it fades in.
 */
export class Limiter {
  private on = false;
  private started = false;
  private mix = 0; // 0 dry .. 1 limited
  private env = 0;
  private hold = 0;
  private holdSamples: number;
  private rel: number;
  private fade: number;

  constructor(sampleRate: number) {
    this.holdSamples = Math.round(LIMIT_HOLD_S * sampleRate);
    this.rel = Math.exp(-1 / (LIMIT_RELEASE_S * sampleRate));
    this.fade = 1 / (LIMIT_FADE_S * sampleRate);
  }

  /** Before the first sample it switches at once (nothing to fade from); after that it crossfades. */
  set(on: boolean) {
    this.on = on;
    if (!this.started) this.mix = on ? 1 : 0;
  }

  /** Writes the (maybe) limited input to out (same length). */
  process(input: Float32Array, out: Float32Array) {
    this.started = true;
    for (let i = 0; i < input.length; i++) {
      let x = input[i];
      let a = Math.abs(x);
      if (!(a <= LIMIT_SANE)) x = a = 0;
      if (a >= this.env) {
        this.env = a;
        this.hold = this.holdSamples;
      } else if (this.hold > 0) this.hold--;
      else this.env = Math.max(a, this.env * this.rel); // never decays below this sample
      const lim = this.env <= LIMIT_KNEE ? x : (x * softCurve(this.env)) / this.env;
      this.mix = this.on ? Math.min(1, this.mix + this.fade) : Math.max(0, this.mix - this.fade);
      out[i] = this.mix === 0 ? x : this.mix === 1 ? lim : x + this.mix * (lim - x);
    }
  }
}

/** Gate timings. Attack and release are linear gain ramps, so a change never makes a step (a click). */
export const GATE_HYSTERESIS_DB = 4;
const GATE_ATTACK_S = 0.002;
const GATE_HOLD_S = 0.2;
const GATE_RELEASE_S = 0.12;

/**
 * Noise gate on the denoiser output. `detect` sees each denoised 10 ms frame when it arrives, and `apply` shapes the
 * samples as they play. The output queue holds about 20 to 80 ms between the two, so the gate opens before a word's
 * first sample plays: that queue is the lookahead, with no extra delay. It opens on a frame peak above the threshold
 * and closes after GATE_HOLD_S of frames below threshold minus the hysteresis. Hold counts from arrival, so in played
 * time the release starts 200 ms minus the queue depth (about 120 to 180 ms) after the sound drops. Off and fully
 * open, it does nothing at all.
 */
export class Gate {
  on = false;
  private open = db(-50);
  private close = db(-50 - GATE_HYSTERESIS_DB);
  private quiet = 0; // samples since the last frame at or above the close level
  private target = 1;
  private gain = 1;
  private holdSamples: number;
  private up: number;
  private down: number;

  constructor(sampleRate: number) {
    this.holdSamples = GATE_HOLD_S * sampleRate;
    this.up = 1 / (GATE_ATTACK_S * sampleRate);
    this.down = 1 / (GATE_RELEASE_S * sampleRate);
  }

  /** Turning it on starts open, so the first word is not cut. Turning it off ramps open. A repeat set (a marker move) keeps the state. */
  set(on: boolean, thresholdDb: number) {
    if (on !== this.on) [this.quiet, this.target] = [0, 1];
    this.on = on;
    this.open = db(thresholdDb);
    this.close = db(thresholdDb - GATE_HYSTERESIS_DB);
  }

  detect(frame: Float32Array) {
    if (!this.on) return;
    let peak = 0;
    for (let i = 0; i < frame.length; i++) peak = Math.max(peak, Math.abs(frame[i]));
    if (peak > this.open) this.target = 1;
    if (peak >= this.close) this.quiet = 0;
    else if ((this.quiet += frame.length) >= this.holdSamples) this.target = 0;
  }

  apply(o: Float32Array) {
    if (!this.on && this.gain === 1) return;
    for (let i = 0; i < o.length; i++) {
      this.gain = this.target > this.gain ? Math.min(1, this.gain + this.up) : Math.max(this.target, this.gain - this.down);
      o[i] *= this.gain;
    }
  }
}
