export const VOLUME_KEY = 'vesper.volumes';

/** The per-person volume slider's top, in percent. */
export const MAX_VOLUME = 400;

/** Slider percent (0 to 400) to linear gain: quadratic to 1.0 at 100, then even dB steps to +20 dB (10x) at 400. */
export function volumeToGain(v: number): number {
  if (Number.isNaN(v)) return 1;
  const c = Math.min(MAX_VOLUME, Math.max(0, v));
  return c <= 100 ? (c / 100) ** 2 : 10 ** ((c - 100) / 300);
}

export function restoreVolumes(store: Storage): Map<string, number> {
  const out = new Map<string, number>();
  try {
    const o = JSON.parse(store.getItem(VOLUME_KEY) ?? '{}');
    if (o && typeof o === 'object' && !Array.isArray(o))
      for (const [k, v] of Object.entries(o)) if (typeof v === 'number' && Number.isFinite(v)) out.set(k, v);
  } catch {
    /* corrupt storage: start fresh */
  }
  return out;
}

export function saveVolumes(store: Storage, m: Map<string, number>): void {
  store.setItem(VOLUME_KEY, JSON.stringify(Object.fromEntries(m)));
}

export const effectiveGain = (volume: number, deafened: boolean) => (deafened ? 0 : volumeToGain(volume));

/** RMS of time-domain samples (-1..1) above a speech threshold. */
export function isSpeaking(samples: Float32Array, threshold = 0.02): boolean {
  let s = 0;
  for (const x of samples) s += x * x;
  return Math.sqrt(s / samples.length) > threshold;
}
