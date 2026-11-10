import { describe, expect, it } from 'vitest';
import { pointIn } from './geometry';

// 1920x1080 in 960x720 fits 960x540, with 90 px bars above and below.
const box = { left: 100, top: 50, width: 960, height: 720 };

describe('pointIn', () => {
  it('maps the centre', () => {
    expect(pointIn(580, 410, box, 1920, 1080)).toEqual({ x: 0.5, y: 0.5 });
  });
  it('is null in the letterbox bar', () => {
    expect(pointIn(580, 100, box, 1920, 1080)).toBeNull();
    expect(pointIn(580, 760, box, 1920, 1080)).toBeNull();
  });
  it('maps the picture top-left', () => {
    expect(pointIn(100, 140, box, 1920, 1080)).toEqual({ x: 0, y: 0 });
  });
  it('is null before the video has a size', () => {
    expect(pointIn(580, 410, box, 0, 0)).toBeNull();
  });
});
