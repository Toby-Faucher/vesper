// The call sounds: short synthesised tones, no audio files. One AudioContext of its own, apart from the call's audio
// pipeline and the mic (it connects only to its own speaker output), opened on the first sound and closed after 10 s
// without one.
import type { Cue } from './cues';

let ctx: AudioContext | null = null;
let idle: ReturnType<typeof setTimeout> | undefined;
let sinkNow: string | null = null; // the sink a new context starts on

type Tone = { hz: number[]; step: number; wave: OscillatorType; gap?: number; gain?: number };
// One family: rising = on (unmute, connected, someone joined), falling = off, a soft same-pitch double for deafen,
// a low pair for a lost connection. Mute cues are high and quick, connection cues use a warmer triangle wave.
export const TONES: Record<Cue, Tone> = {
  // Others joining or leaving: 2.5 times the peak (+8 dB), longer and brighter, so they carry over game audio.
  join: { hz: [523.25, 783.99], step: 0.12, wave: 'triangle', gain: 2.5 }, // C5 G5
  leave: { hz: [783.99, 523.25], step: 0.12, wave: 'triangle', gain: 2.5 },
  unmute: { hz: [880, 1318.5], step: 0.06, wave: 'sine' }, // A5 E6
  mute: { hz: [1318.5, 880], step: 0.06, wave: 'sine' },
  deafen: { hz: [440, 440], step: 0.06, wave: 'sine', gap: 0.03 }, // A4 twice
  undeafen: { hz: [659.25, 659.25], step: 0.06, wave: 'sine', gap: 0.03 }, // E5 twice
  connect: { hz: [587.33, 880], step: 0.11, wave: 'triangle' }, // D5 A5
  disconnect: { hz: [880, 587.33], step: 0.11, wave: 'triangle' },
  ring: { hz: [659.25, 523.25, 659.25, 523.25], step: 0.16, wave: 'triangle', gap: 0.04 }, // E5 C5 twice, an incoming DM call
  'share-start': { hz: [698.46, 1046.5], step: 0.07, wave: 'triangle' }, // F5 C6
  'share-stop': { hz: [1046.5, 698.46], step: 0.07, wave: 'triangle' },
  lost: { hz: [220, 164.81], step: 0.12, wave: 'triangle' }, // A3 E3
};
/** Peak gain at volume 100; the default 50 gives 0.08 (about -22 dBFS). */
export const MAX_GAIN = 0.16;
export const toneMs = (t: Tone) => Math.round(1000 * (t.hz.length * t.step + (t.hz.length - 1) * (t.gap ?? 0)));

/** `volume` is 0 to 100. */
export function chime(kind: Cue, sinkId: string | null, volume = 50): void {
  const level = (MAX_GAIN * Math.min(100, Math.max(0, volume))) / 100;
  if (!level) return;
  try {
    if (ctx?.state === 'closed') [ctx, sinkNow] = [null, null]; // closed under us: start a new one
    ctx ??= new AudioContext();
    if (ctx.state === 'suspended') void ctx.resume().catch(() => {}); // no user gesture yet: the tones are queued until it runs
    if (sinkId !== sinkNow) {
      sinkNow = sinkId;
      const sink = ctx as AudioContext & { setSinkId?: (id: string) => Promise<void> };
      sink.setSinkId?.(sinkId ?? '').catch(() => {}); // '' is the default speaker; a failure keeps the current one
    }
    const tone = TONES[kind];
    const peak = level * (tone.gain ?? 1);
    const t0 = ctx.currentTime + 0.01;
    tone.hz.forEach((hz, i) => {
      const osc = ctx!.createOscillator();
      const gain = ctx!.createGain();
      const t = t0 + i * (tone.step + (tone.gap ?? 0));
      osc.type = tone.wave;
      osc.frequency.value = hz;
      gain.gain.setValueAtTime(0, t);
      gain.gain.linearRampToValueAtTime(peak, t + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + tone.step);
      osc.connect(gain).connect(ctx!.destination);
      osc.start(t);
      osc.stop(t + tone.step);
    });
  } catch {
    return; // no Web Audio: no sound
  }
  clearTimeout(idle);
  idle = setTimeout(() => {
    void ctx?.close().catch(() => {});
    [ctx, sinkNow] = [null, null];
  }, 10000);
}
