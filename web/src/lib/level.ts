// "Even out voices": a slow gain per remote person that brings loud and quiet friends to a common level. Listener side only:
// it reads the playback analyser (before any gain) and sets a gain node under the manual volume. The mic path is not touched.

export const LEVEL_KEY = 'vesper.level';
/** Speech level everyone is brought to, in dBFS RMS of a 21 ms analyser frame. */
export const TARGET_DB = -28;
/** The most the levelling may lift or cut. */
export const MAX_DB = 10;
/** Frames below this are silence or noise: they are not measured and the gain holds. */
export const FLOOR_DB = -50;
/** The speech level is a running average over about this many seconds of speech. */
export const WINDOW_S = 3;
/** Seconds of speech heard before the first change. */
export const SETTLE_S = 1.5;
/** Time constants of the gain: a cut (someone got loud) is quicker than a lift, so a shout is tamed and a pause does not pump. */
export const ATTACK_S = 1;
export const RELEASE_S = 4;

export type LevelState = { levelDb: number | null; heardS: number; gainDb: number };
export const LEVEL_START: LevelState = { levelDb: null, heardS: 0, gainDb: 0 };

export const rmsDb = (x: Float32Array): number => {
  let s = 0;
  for (const v of x) s += v * v;
  return s > 0 ? 10 * Math.log10(s / x.length) : -Infinity;
};
export const dbToGain = (db: number) => 10 ** (db / 20);

const ease = (from: number, to: number, dt: number, tau: number) => from + (to - from) * (1 - Math.exp(-dt / tau));

/** One analyser frame `frameDb` that stands for `dt` seconds. Silence, noise and a bad frame change nothing. */
export function stepLevel(s: LevelState, frameDb: number, dt: number): LevelState {
  if (!Number.isFinite(frameDb) || !(dt > 0) || frameDb < FLOOR_DB) return s;
  const levelDb = s.levelDb === null ? frameDb : ease(s.levelDb, frameDb, dt, WINDOW_S);
  const heardS = s.heardS + dt;
  if (heardS < SETTLE_S) return { levelDb, heardS, gainDb: s.gainDb };
  const target = Math.min(MAX_DB, Math.max(-MAX_DB, TARGET_DB - levelDb));
  return { levelDb, heardS, gainDb: ease(s.gainDb, target, dt, target < s.gainDb ? ATTACK_S : RELEASE_S) };
}

const storage = (() => { try { return localStorage; } catch { return null; } })();
/** Off unless turned on. */
export const loadLevelling = (): boolean => storage?.getItem(LEVEL_KEY) === 'on';
export function saveLevelling(on: boolean): void {
  try { storage?.setItem(LEVEL_KEY, on ? 'on' : 'off'); } catch { /* storage blocked: the setting lasts this page */ }
}
