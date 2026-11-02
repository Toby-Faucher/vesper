import { describe, expect, it } from 'vitest';
import { ATTACK_S, FLOOR_DB, LEVEL_START, MAX_DB, SETTLE_S, TARGET_DB, rmsDb, stepLevel, type LevelState } from './level';

const DT = 0.1; // the speaking poll
/** Feeds `seconds` of frames from `db(t)` and returns the state after each frame. */
function feed(db: (t: number) => number, seconds: number, from: LevelState = LEVEL_START): LevelState[] {
  const out: LevelState[] = [];
  let s = from;
  for (let i = 0; i < seconds / DT; i++) out.push((s = stepLevel(s, db(i * DT), DT)));
  return out;
}
const last = (x: LevelState[]) => x[x.length - 1];
// Speech: a syllable rhythm of about 4 Hz around `db`, with short gaps under the floor.
const talker = (db: number) => (t: number) => (Math.floor(t * 10) % 5 === 4 ? -80 : db + 4 * Math.sin(t * 25));

describe('stepLevel', () => {
  it.each([
    ['on target', TARGET_DB, 0],
    ['6 dB quiet', TARGET_DB - 6, 6],
    ['6 dB loud', TARGET_DB + 6, -6],
    ['very quiet: capped', TARGET_DB - 20, MAX_DB],
    ['very loud: capped', TARGET_DB + 20, -MAX_DB],
  ])('a steady %s talker settles at %d dB', (_, level, want) => {
    expect(last(feed(() => level, 60)).gainDb).toBeCloseTo(want, 1);
  });

  it('a loud and a quiet talker converge within the cap', () => {
    const loud = last(feed(talker(-18), 40));
    const quiet = last(feed(talker(-40), 40));
    const out = (s: LevelState, db: number) => db + s.gainDb;
    expect(Math.abs(out(loud, -18) - out(quiet, -40))).toBeLessThan(4); // 22 dB apart before
    for (const s of [loud, quiet]) expect(Math.abs(s.gainDb)).toBeLessThanOrEqual(MAX_DB);
  });

  it('silence and noise under the floor do not drift the gain', () => {
    const talked = last(feed(talker(-40), 30));
    const after = last(feed(() => FLOOR_DB - 1, 120, talked));
    expect(after).toEqual(talked);
    expect(last(feed(() => -Infinity, 30))).toEqual(LEVEL_START); // digital silence from the start
  });

  it('waits for some speech before the first change', () => {
    const early = feed(() => TARGET_DB - 10, SETTLE_S - DT);
    expect(early.every((s) => s.gainDb === 0)).toBe(true);
  });

  it('a step change is smoothed over seconds, not jumped', () => {
    const settled = last(feed(() => TARGET_DB, 30));
    const steps = feed(() => TARGET_DB + 12, 10, settled); // someone starts shouting
    const moves = steps.map((s, i) => Math.abs(s.gainDb - (i ? steps[i - 1].gainDb : settled.gainDb)));
    expect(Math.max(...moves)).toBeLessThan(0.5); // no frame moves the gain by half a dB
    expect(steps[Math.round(ATTACK_S / DT)].gainDb).toBeGreaterThan(-6); // still on the way after one attack time
    expect(last(steps).gainDb).toBeLessThan(-8); // and most of the way there after 10 s
  });

  it('cuts faster than it lifts, so pauses and soft words do not pump', () => {
    const settled = last(feed(() => TARGET_DB, 30));
    const cut = last(feed(() => TARGET_DB + 8, 2, settled)).gainDb;
    const lift = last(feed(() => TARGET_DB - 8, 2, settled)).gainDb;
    expect(-cut).toBeGreaterThan(lift * 1.5);
  });

  it('never gives NaN', () => {
    for (const db of [NaN, Infinity, -Infinity, 0, 1e9, -1e9])
      for (const dt of [0, -1, NaN, 0.1, 1e6]) {
        const s = stepLevel({ levelDb: -30, heardS: 5, gainDb: 2 }, db, dt);
        for (const v of Object.values(s)) expect(Number.isNaN(v)).toBe(false);
        expect(Math.abs(s.gainDb)).toBeLessThanOrEqual(MAX_DB);
      }
  });
});

it('rmsDb of a full-scale sine is about -3 dB; silence is -Infinity', () => {
  const x = new Float32Array(1024).map((_, i) => Math.sin((2 * Math.PI * i) / 64));
  expect(rmsDb(x)).toBeCloseTo(-3.01, 1);
  expect(rmsDb(new Float32Array(1024))).toBe(-Infinity);
});
