import { expect, it } from 'vitest';
import { figures, shareLine } from './line';

it('rounds_the_rate_and_shows_the_viewers', () => {
  expect(shareLine({ width: 2560, height: 1440, fps: 29.6, kbps: 7_540, watchers: 0 })).toBe('2560x1440 · 30 fps · 7.5 Mbps · 0 watching');
  expect(shareLine({ width: 0, height: 0, fps: 0, kbps: 0, watchers: 1 })).toBe('Starting · 1 watching');
});

it('figures_without_viewers_for_a_tile', () => {
  expect(figures({ width: 1920, height: 1080, fps: 59.9, kbps: 12_000 })).toBe('1920x1080 · 60 fps · 12 Mbps');
});
