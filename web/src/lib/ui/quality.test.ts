import { expect, it } from 'vitest';
import { quality, qualityText } from './quality';

const d = (rttMs: number | null, lossDownPct: number | null = 0, lossUpPct: number | null = 0) => ({ rttMs, lossDownPct, lossUpPct });

it('thresholds_at_the_edges', () => {
  expect(quality(d(149.9, 1.99, 0))).toBe('good');
  expect(quality(d(150))).toBe('fair');
  expect(quality(d(0, 2))).toBe('fair');
  expect(quality(d(299.9, 4.99))).toBe('fair');
  expect(quality(d(300))).toBe('poor');
  expect(quality(d(0, 5))).toBe('poor');
});

it('nulls_give_unknown', () => {
  expect(quality(null)).toBe('unknown');
  expect(quality(d(null, null, null))).toBe('unknown');
  // A missing value does not count against the call.
  expect(quality(d(null, 0, null))).toBe('good');
  expect(quality(d(40, null, null))).toBe('good');
});

it('the_worse_of_up_and_down_loss_counts', () => {
  expect(quality(d(20, 0, 3))).toBe('fair');
  expect(quality(d(20, 6, 0))).toBe('poor');
  expect(quality(d(20, 1, 6))).toBe('poor');
});

it('text_only_when_fair_or_poor', () => {
  expect(qualityText('good', d(20))).toBe('');
  expect(qualityText('unknown', null)).toBe('');
  expect(qualityText('fair', d(20, 1, 4.2))).toBe('Weak connection: 4 % loss');
  expect(qualityText('poor', d(320, 0, 0))).toBe('Weak connection: 320 ms delay');
  // Loss over its limit is named before the delay.
  expect(qualityText('poor', d(320, 6, 0))).toBe('Weak connection: 6 % loss');
});
