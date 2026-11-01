import { describe, expect, it } from 'vitest';
import { BOX, fileSize, fit, typeLabel, viewCase } from './attachments';

describe('attachments', () => {
  it('fits an image into the box without scaling it up', () => {
    expect(fit(720, 280)).toEqual({ w: 360, h: 140 });
    expect(fit(300, 1000)).toEqual({ w: 84, h: 280 });
    expect(fit(200, 100)).toEqual({ w: 200, h: 100 });
    expect(fit(1, 1)).toEqual({ w: 48, h: 48 });
    expect(fit(0, 0)).toEqual(BOX);
  });
  it('labels sniffed images by type and other files by the extension of the name', () => {
    expect(typeLabel('image/webp', 'x.png')).toBe('WebP image');
    expect(typeLabel('application/octet-stream', 'notes.txt')).toBe('TXT file');
    expect(typeLabel('application/octet-stream', 'Makefile')).toBe('');
    expect(typeLabel('application/octet-stream', 'a.verylongext')).toBe('');
  });
  // The list's top is at 100. An image whose row starts above the view but which is itself inside it needs nothing.
  it('keepView: the image top decides, not its row', () => {
    expect(viewCase(500, -300, 100)).toBe('above'); // fully above
    expect(viewCase(500, 60, 100)).toBe('above'); // across the top edge: what is below it would move
    expect(viewCase(500, 140, 100)).toBe('none'); // inside the view
    expect(viewCase(500, 2000, 100)).toBe('none'); // below the view
    expect(viewCase(4, -300, 100)).toBe('bottom'); // at the bottom it stays there
  });
  it('sizes files in KB and MB', () => {
    expect(fileSize(0)).toBe('1 KB');
    expect(fileSize(5 * 1024 * 1024)).toBe('5.0 MB');
    expect(fileSize(25 * 1024 * 1024 + 1)).toBe('25.1 MB');
  });
});
