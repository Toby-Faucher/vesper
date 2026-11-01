// The join and leave sounds: two short sine tones, rising for a join and falling for a leave. Its own AudioContext,
// apart from the call's audio pipeline, opened on the first sound and closed after 10 s without one.
let ctx: AudioContext | null = null;
let idle: ReturnType<typeof setTimeout> | undefined;
let sinkNow: string | null = null; // the sink a new context starts on
const TONES = { join: [523.25, 783.99], leave: [783.99, 523.25] }; // C5 and G5
const GAIN = 0.08;
const STEP = 0.09; // seconds per tone

export function chime(kind: 'join' | 'leave', sinkId: string | null): void {
  try {
    ctx ??= new AudioContext();
    if (sinkId !== sinkNow) {
      sinkNow = sinkId;
      const sink = ctx as AudioContext & { setSinkId?: (id: string) => Promise<void> };
      sink.setSinkId?.(sinkId ?? '').catch(() => {}); // '' is the default speaker; a failure keeps the current one
    }
    const t0 = ctx.currentTime + 0.01;
    TONES[kind].forEach((hz, i) => {
      const osc = ctx!.createOscillator();
      const gain = ctx!.createGain();
      const t = t0 + i * STEP;
      osc.frequency.value = hz;
      gain.gain.setValueAtTime(0, t);
      gain.gain.linearRampToValueAtTime(GAIN, t + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + STEP);
      osc.connect(gain).connect(ctx!.destination);
      osc.start(t);
      osc.stop(t + STEP);
    });
  } catch {
    return; // no Web Audio: no sound
  }
  clearTimeout(idle);
  idle = setTimeout(() => {
    void ctx?.close();
    [ctx, sinkNow] = [null, null];
  }, 10000);
}
