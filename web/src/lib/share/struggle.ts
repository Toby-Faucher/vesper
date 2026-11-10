export type RecvSample = { packetsLost: number; packetsReceived: number; freezeCount: number };

const WINDOW = 4, LOSS = 0.02;

/** Over the last 4 samples (6 s at a 2 s poll): loss above 2%, or a freeze. Counters are cumulative. */
export function struggling(samples: RecvSample[]): boolean {
  if (samples.length < WINDOW) return false;
  const a = samples[samples.length - WINDOW], b = samples[samples.length - 1];
  const lost = b.packetsLost - a.packetsLost;
  const total = lost + (b.packetsReceived - a.packetsReceived);
  return b.freezeCount > a.freezeCount || (total > 0 && lost / total > LOSS);
}
