import { describe, expect, it } from 'vitest';
import { COOLDOWN_MS, GRACE_MS, MUTED_QUIET, SHOW_MS, SUSTAIN_MS, mutedSpeech, type MutedSpeech } from './muted-speech';

const POLL = 100; // the call's poll
/** Polls from `s` for `ms`, each with `speech(i)`; returns the state and when the notice first showed (ms from the start). */
function run(s: MutedSpeech, t0: number, ms: number, speech: (i: number) => boolean, listening = true) {
  let shown: number | null = null;
  for (let i = 0, t = t0; t < t0 + ms; i++, t += POLL) {
    s = mutedSpeech(s, t, { listening, speech: speech(i) });
    if (s.show && shown === null) shown = t - t0;
  }
  return { s, shown };
}
const always = () => true;
const never = () => false;

describe('mutedSpeech', () => {
  it('shows after the grace and half a second of speech, and hides after SHOW_MS', () => {
    const { s, shown } = run(MUTED_QUIET, 0, GRACE_MS + 2000, always);
    expect(shown).toBe(GRACE_MS + SUSTAIN_MS);
    expect(s.show).toBe(true);
    const later = run(s, GRACE_MS + 2000, SHOW_MS, always).s;
    expect(later.show).toBe(false);
  });

  it('ignores speech in the grace after muting: the end of a sentence', () => {
    const { s, shown } = run(MUTED_QUIET, 0, GRACE_MS, always);
    expect(shown).toBeNull();
    expect(s.heard).toBe(0);
  });

  it('never shows for short sounds or clicks', () => {
    // One poll of sound in three, for a minute: quiet takes back more than the sound adds.
    expect(run(MUTED_QUIET, 0, 60_000, (i) => i % 3 === 0).shown).toBeNull();
    // Four polls of sound, then quiet, over and over: never half a second in a row net of the quiet.
    expect(run(MUTED_QUIET, 0, 60_000, (i) => i % 10 < 4).shown).toBeNull();
    expect(run(MUTED_QUIET, 0, 60_000, never).shown).toBeNull();
  });

  it('shows for speech with short pauses', () => {
    // Words with gaps: 3 polls of sound, 1 of quiet.
    expect(run(MUTED_QUIET, 0, GRACE_MS + 3000, (i) => i % 4 !== 3).shown).not.toBeNull();
  });

  it('waits COOLDOWN_MS before the next notice, across unmute and mute', () => {
    let { s } = run(MUTED_QUIET, 0, GRACE_MS + SUSTAIN_MS + POLL, always);
    const first = s.shownAt!;
    s = run(s, GRACE_MS + SUSTAIN_MS + POLL, 1000, always, false).s; // unmuted: hidden at once
    expect(s.show).toBe(false);
    const mid = run(s, 3000, 30_000, always);
    expect(mid.shown).toBeNull();
    const after = run(mid.s, 33_000, COOLDOWN_MS, always);
    expect(after.shown).not.toBeNull();
    expect(after.s.shownAt! - first).toBeGreaterThanOrEqual(COOLDOWN_MS);
  });

  it('unmuting hides it at once and starts the grace again', () => {
    let { s } = run(MUTED_QUIET, 0, GRACE_MS + SUSTAIN_MS + POLL, always);
    expect(s.show).toBe(true);
    s = mutedSpeech(s, 2000, { listening: false, speech: true });
    expect(s).toMatchObject({ show: false, heard: 0, from: null });
  });

  it('a throttled timer cannot show it from one loud poll', () => {
    let s = mutedSpeech(MUTED_QUIET, 0, { listening: true, speech: false });
    s = mutedSpeech(s, 2000, { listening: true, speech: false });
    s = mutedSpeech(s, 3000, { listening: true, speech: true }); // a second since the last poll
    expect(s.show).toBe(false);
    expect(s.heard).toBe(200);
  });
});
