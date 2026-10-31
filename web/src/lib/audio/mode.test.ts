import { describe, expect, it } from 'vitest';
import { ModeController } from './mode';

const make = () => new ModeController({ budgetMs: 10, windowFrames: 20, maxLate: 5 });

describe('ModeController', () => {
  it('10 frames at 2 ms stay deepfilter', () => {
    const m = make();
    for (let i = 0; i < 10; i++) expect(m.report(2)).toBe('deepfilter');
  });
  it('5 of 20 frames at 15 ms switch to browser and stay until reset', () => {
    const m = make();
    const modes = Array.from({ length: 20 }, (_, i) => m.report(i % 4 === 0 ? 15 : 2));
    expect(modes.at(-1)).toBe('browser');
    expect(modes.slice(0, 16)).not.toContain('browser'); // 5th late frame is #16
    for (let i = 0; i < 50; i++) expect(m.report(1)).toBe('browser');
    m.reset();
    expect(m.report(1)).toBe('deepfilter');
  });
  it('late frames that leave the window are forgotten', () => {
    const m = make();
    for (let k = 0; k < 10; k++) {
      for (let i = 0; i < 4; i++) m.report(15);
      for (let i = 0; i < 20; i++) expect(m.report(2)).toBe('deepfilter');
    }
  });
  it('ignores frames during warm-up, even a burst of slow ones', () => {
    const m = new ModeController({ budgetMs: 30, windowFrames: 100, maxLate: 20, warmupFrames: 300 });
    for (let i = 0; i < 50; i++) expect(m.report(80)).toBe('deepfilter');
    for (let i = 0; i < 250; i++) expect(m.report(80)).toBe('deepfilter'); // still warm-up, nothing stored
    for (let i = 0; i < 19; i++) expect(m.report(31)).toBe('deepfilter'); // window starts empty after warm-up
  });
  it('after warm-up, 19 slow frames of 100 do not trip and the 20th does', () => {
    const m = new ModeController({ budgetMs: 30, windowFrames: 100, maxLate: 20, warmupFrames: 300 });
    for (let i = 0; i < 300; i++) m.report(1);
    for (let i = 0; i < 81; i++) m.report(1);
    for (let i = 0; i < 19; i++) expect(m.report(31)).toBe('deepfilter');
    expect(m.report(31)).toBe('browser');
  });
  it('frames of exactly the budget and under never count as late', () => {
    const m = new ModeController({ budgetMs: 30, windowFrames: 100, maxLate: 20 });
    for (let i = 0; i < 500; i++) expect(m.report(i % 2 ? 30 : 29)).toBe('deepfilter');
  });
});
