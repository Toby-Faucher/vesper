import { RING_EVERY_MS } from '../ring.svelte';
import { describe, expect, it } from 'vitest';
import { callCue, type CallSnap, type Cue } from './cues';
import { TONES, MAX_GAIN, toneMs } from './chime';

const all = { call: true, muteCues: true, connectCues: true };
const idle: CallSnap = { joined: false, muted: false, deafened: false, online: true };
const inCall: CallSnap = { ...idle, joined: true };
const s = (o: Partial<CallSnap>): CallSnap => ({ ...inCall, ...o });

// Plays a run of states through callCue, as the voice effect does, and lists the cues.
function run(states: CallSnap[], prefs = all): Cue[] {
  let was: CallSnap | null = null;
  const out: Cue[] = [];
  for (const now of states) {
    const c = callCue(was, now, prefs);
    if (c) out.push(c);
    was = now;
  }
  return out;
}

describe('callCue', () => {
  it.each<[string, CallSnap, CallSnap, Cue | null]>([
    ['join', idle, inCall, 'connect'],
    ['leave', inCall, idle, 'disconnect'],
    ['socket drop ends the call', inCall, { ...idle, online: false }, 'lost'],
    ['mute', inCall, s({ muted: true }), 'mute'],
    ['unmute', s({ muted: true }), inCall, 'unmute'],
    ['deafen', inCall, s({ deafened: true }), 'deafen'],
    ['undeafen', s({ deafened: true }), inCall, 'undeafen'],
    ['mute while deafened is silent', s({ deafened: true }), s({ deafened: true, muted: true }), null],
    ['leave while muted and deafened: only disconnect', s({ muted: true, deafened: true }), idle, 'disconnect'],
    ['joined already muted (a force-mute on join): only connect', idle, s({ muted: true }), 'connect'],
    ['socket back while not in a call', { ...idle, online: false }, idle, null],
    ['socket flaps in a call (no change)', inCall, s({ online: false }), null],
    ['flags change outside a call', idle, { ...idle, muted: true }, null],
    ['no change', inCall, inCall, null],
  ])('%s', (_, prev, next, cue) => expect(callCue(prev, next, all)).toBe(cue));

  it('is silent on the first look, whatever the state (page load)', () => {
    for (const st of [idle, inCall, s({ muted: true }), s({ deafened: true })]) expect(callCue(null, st, all)).toBe(null);
  });

  it('a force-mute plays the mute cue once, its repeat and its lift none', () => {
    // force_muted true (muted), sent again after a reconnect of the server (same state), then lifted (still muted).
    expect(run([inCall, s({ muted: true }), s({ muted: true }), s({ muted: true })])).toEqual(['mute']);
  });

  it('a lost connection plays once, reconnect flaps after it play nothing', () => {
    const lost = { ...idle, online: false };
    expect(run([inCall, lost, idle, lost, idle, lost])).toEqual(['lost']);
  });

  it('a full call: join, mute, unmute, deafen, undeafen, leave', () => {
    expect(run([idle, inCall, s({ muted: true }), inCall, s({ deafened: true }), inCall, idle])).toEqual([
      'connect', 'mute', 'unmute', 'deafen', 'undeafen', 'disconnect',
    ]);
  });

  it('each switch turns its group off, the master all of them', () => {
    const states = [idle, inCall, s({ muted: true }), s({ muted: true, deafened: true }), idle];
    expect(run(states, { ...all, muteCues: false })).toEqual(['connect', 'disconnect']);
    expect(run(states, { ...all, connectCues: false })).toEqual(['mute', 'deafen']);
    expect(run(states, { ...all, call: false })).toEqual([]);
  });
});

describe('the tones', () => {
  it.each(Object.entries(TONES).filter(([k]) => k !== 'ring'))('%s is 60 to 250 ms', (_, t) => {
    expect(toneMs(t)).toBeGreaterThanOrEqual(60);
    expect(toneMs(t)).toBeLessThanOrEqual(250);
  });
  // The ring repeats every 2.5 s while a prompt shows, so it is a short phrase of its own: under a second, with a gap left to repeat.
  it('the ring is under 1 s and shorter than its repeat', () => {
    expect(toneMs(TONES.ring)).toBeLessThanOrEqual(1000);
    expect(toneMs(TONES.ring)).toBeLessThan(RING_EVERY_MS);
  });
  it('the default volume peaks below -18 dBFS', () => expect(20 * Math.log10(MAX_GAIN / 2)).toBeLessThan(-18));
});
