import { describe, expect, it } from 'vitest';
import { struggling, type RecvSample } from './struggle';

const s = (packetsLost: number, packetsReceived: number, freezeCount = 0): RecvSample => ({ packetsLost, packetsReceived, freezeCount });

describe('struggling', () => {
  it('is false with fewer than 4 samples', () => {
    expect(struggling([s(0, 0), s(50, 50), s(100, 100)])).toBe(false);
  });
  it('is true when loss over the last 4 samples is above 2%', () => {
    expect(struggling([s(0, 0), s(1, 100), s(2, 200), s(8, 300)])).toBe(true);
  });
  it('ignores old loss outside the window', () => {
    expect(struggling([s(0, 0), s(500, 500), s(500, 600), s(500, 700), s(500, 800)])).toBe(false);
  });
  it('is true when freezeCount grew', () => {
    expect(struggling([s(0, 0), s(0, 100), s(0, 200, 1), s(0, 300, 1)])).toBe(true);
  });
  it('is false on a clean run', () => {
    expect(struggling([s(0, 0, 2), s(0, 100, 2), s(1, 200, 2), s(1, 300, 2)])).toBe(false);
  });
});
