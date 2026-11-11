import { describe, expect, it } from 'vitest';
import { fit, pictureBox, pointIn } from './geometry';

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

describe('pictureBox', () => {
  it('puts the bars above and below a wide picture', () => {
    expect(pictureBox(960, 720, 1920, 1080)).toEqual({ left: 0, top: 90, width: 960, height: 540 });
  });
  it('puts the bars left and right of a tall picture', () => {
    expect(pictureBox(1600, 900, 1080, 1080)).toEqual({ left: 350, top: 0, width: 900, height: 900 });
  });
  it('is the whole box before the video has a size', () => {
    expect(pictureBox(960, 720, 0, 0)).toEqual({ left: 0, top: 0, width: 960, height: 720 });
  });
  it('round trips with pointIn', () => {
    const b = pictureBox(960, 720, 1920, 1080);
    expect(pointIn(100 + b.left + b.width / 4, 50 + b.top + b.height / 2, box, 1920, 1080)).toEqual({ x: 0.25, y: 0.5 });
  });
});

describe('fit', () => {
  it('one share takes the whole width of a wide stage, or its height', () => {
    expect(fit(1, 1600, 900, 12)).toEqual({ cols: 1, width: 1600 });
    expect(fit(1, 1600, 450, 12)).toEqual({ cols: 1, width: 800 });
  });
  it('two shares in a wide, short stage sit side by side', () => {
    expect(fit(2, 1600, 400, 12)).toEqual({ cols: 2, width: 711 });
  });
  it('two shares in a tall stage stack', () => {
    expect(fit(2, 800, 1000, 12)).toEqual({ cols: 1, width: 800 });
  });
  it('four shares make a 2 by 2 grid in a 16:9 stage', () => {
    expect(fit(4, 1612, 912, 12)).toEqual({ cols: 2, width: 800 });
  });
  it('maxCols 1 stacks them (dock)', () => {
    expect(fit(2, 1600, 400, 12, 1)).toEqual({ cols: 1, width: 344 });
  });
  it('never goes below zero in a stage with no room', () => {
    expect(fit(3, 0, 0, 12).width).toBe(0);
  });
});
