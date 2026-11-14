import { describe, expect, it } from 'vitest';
import type { Attachment } from '../gen/Attachment';
import type { Message } from '../gen/Message';
import { BOX, fileSize, fit, isGenericPaste, pastedName, typeLabel, viewable, viewCase } from './attachments';

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

  it('names a pasted image by local time, with a counter, and keeps the type', () => {
    const t = new Date(2026, 10, 3, 14, 5, 59);
    expect(pastedName('image/png', t, new Set())).toBe('Pasted image 2026-11-03 14.05.png');
    const taken = new Set(['Pasted image 2026-11-03 14.05.png']);
    expect(pastedName('image/png', t, taken)).toBe('Pasted image 2026-11-03 14.05 (2).png');
    taken.add('Pasted image 2026-11-03 14.05 (2).png');
    expect(pastedName('image/png', t, taken)).toBe('Pasted image 2026-11-03 14.05 (3).png');
    expect(pastedName('image/jpeg', new Date(2026, 0, 2, 3, 4), new Set())).toBe('Pasted image 2026-01-02 03.04.jpg');
    expect(pastedName('image/webp', t, new Set())).toMatch(/\.webp$/);
    expect(pastedName('image/gif', t, new Set())).toMatch(/\.gif$/);
  });

  it('only renames the generic clipboard name', () => {
    expect(isGenericPaste('image.png')).toBe(true);
    expect(isGenericPaste('Image.JPG')).toBe(true);
    expect(isGenericPaste('holiday.png')).toBe(false);
    expect(isGenericPaste('my image.png')).toBe(false);
  });

  it('the viewer walks images and videos in order, not audio, files or deleted messages', () => {
    const a = (id: string, kind: Attachment['kind']) => ({ id, name: id, mime: '', size: 1, kind });
    const m = (attachments: Attachment[], deleted = false) => ({ attachments, deleted }) as Message;
    const list = viewable([m([a('i1', 'image'), a('s1', 'audio')]), m([a('v1', 'video')], true), m([a('f1', 'file'), a('v2', 'video'), a('i2', 'image')])]);
    expect(list.map((x) => x.id)).toEqual(['i1', 'v2', 'i2']);
  });
});
