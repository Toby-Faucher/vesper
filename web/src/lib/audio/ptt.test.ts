import { describe, expect, it } from 'vitest';
import { pttKeyEvent, type PttEvent } from './ptt';

const ev = (o: Partial<PttEvent>): PttEvent =>
  ({ type: 'keydown', code: 'Space', pttKey: 'Space', enabled: true, typing: false, capturing: false, ...o });

describe('pttKeyEvent', () => {
  it('press sets held and prevents default, also with focus on a button', () => {
    expect(pttKeyEvent(false, ev({}))).toEqual({ held: true, prevent: true });
  });
  it('keydown while typing does not set held', () => {
    expect(pttKeyEvent(false, ev({ typing: true })).held).toBe(false);
  });
  it('keyup in an input still clears held', () => {
    expect(pttKeyEvent(true, ev({ type: 'keyup', typing: true })).held).toBe(false);
  });
  it('keyup clears held even if PTT was just disabled', () => {
    expect(pttKeyEvent(true, ev({ type: 'keyup', enabled: false })).held).toBe(false);
  });
  it('capture mode and other keys leave state alone', () => {
    expect(pttKeyEvent(false, ev({ capturing: true }))).toEqual({ held: false, prevent: false });
    expect(pttKeyEvent(true, ev({ code: 'KeyA', type: 'keyup' }))).toEqual({ held: true, prevent: false });
  });
  it('a saved Tab key talks but never swallows Tab', () => {
    expect(pttKeyEvent(false, ev({ code: 'Tab', pttKey: 'Tab' }))).toEqual({ held: true, prevent: false });
  });
  it('disabled PTT ignores presses', () => {
    expect(pttKeyEvent(false, ev({ enabled: false })).held).toBe(false);
  });
});
