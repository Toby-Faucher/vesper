import { expect, it } from 'vitest';
import { clock } from './clock';

it('clock_counts_minutes_then_hours', () => {
  expect(clock(-5)).toBe('0:00');
  expect(clock(999)).toBe('0:00');
  expect(clock(61_000)).toBe('1:01');
  expect(clock(3_599_000)).toBe('59:59');
  expect(clock(3_600_000)).toBe('1:00:00');
  expect(clock(36_061_000)).toBe('10:01:01');
});
