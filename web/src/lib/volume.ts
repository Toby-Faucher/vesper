export const VOLUME_KEY = 'vesper.volumes';

/** Slider percent (0 to 200) to linear gain: quadratic to 1.0 at 100, linear to 2.5 at 200. */
export function volumeToGain(v: number): number {
  if (Number.isNaN(v)) return 1;
  const c = Math.min(200, Math.max(0, v));
  return c <= 100 ? (c / 100) ** 2 : 1 + ((c - 100) / 100) * 1.5;
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
