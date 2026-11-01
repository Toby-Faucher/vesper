import { describe, expect, it } from 'vitest';
import { BOX, fileSize, fit, typeLabel } from './attachments';

describe('attachments', () => {
  it('fits an image into the box without scaling it up', () => {
    expect(fit(720, 280)).toEqual({ w: 360, h: 140 });
    expect(fit(300, 1000)).toEqual({ w: 84, h: 280 });
    expect(fit(200, 100)).toEqual({ w: 200, h: 100 });
    expect(fit(1, 1)).toEqual({ w: 48, h: 48 });
    expect(fit(0, 0)).toEqual(BOX);
  });
  it('labels only the types the server sniffs', () => {
    expect(typeLabel('image/webp')).toBe('WebP image');
    expect(typeLabel('application/octet-stream')).toBe('File');
  });
  it('sizes files in KB and MB', () => {
    expect(fileSize(0)).toBe('1 KB');
    expect(fileSize(5 * 1024 * 1024)).toBe('5.0 MB');
    expect(fileSize(25 * 1024 * 1024 + 1)).toBe('25.1 MB');
  });
});
