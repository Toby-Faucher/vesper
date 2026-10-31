import { dbfs } from './level';

const SILENT_DBFS = -60;
const SILENT_MS = 5000;

export type Silence = { since: number | null; show: boolean };
export const MIC_QUIET: Silence = { since: null, show: false };

/**
 * One poll of the "no sound from your microphone" check. inputRms is the pipeline's input level; a worklet with no
 * input at all reports it as 0, so it counts as silence too. null (no pipeline data) never counts. gated is mute or PTT.
 */
export function micSilence(prev: Silence, now: number, inputRms: number | null, gated: boolean): Silence {
  const quiet = !gated && inputRms !== null && dbfs(inputRms) < SILENT_DBFS;
  const since = quiet ? (prev.since ?? now) : null;
  return { since, show: since !== null && now - since >= SILENT_MS };
}
