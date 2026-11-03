import { describe, expect, it } from 'vitest';
import { fromBase64Url, toBase64Url } from './base64url';

describe('base64url', () => {
  it('round-trips every length from 0 to 40, with bytes that need - and _', () => {
    for (let n = 0; n <= 40; n++) {
      const bytes = Uint8Array.from({ length: n }, (_, i) => (i * 37 + 251) & 0xff);
      const text = toBase64Url(bytes);
      expect(text).not.toMatch(/[+/=]/);
      expect([...fromBase64Url(text)]).toEqual([...bytes]);
    }
    expect(toBase64Url(new Uint8Array([0xfb, 0xff, 0xfe]))).toBe('-__-');
  });

  it('takes an ArrayBuffer', () => {
    expect(toBase64Url(new Uint8Array([104, 105]).buffer)).toBe('aGk');
  });

  it('refuses text that is not base64url', () => {
    for (const bad of ['a+b/', 'aGk=', 'a', 'a b!']) expect(() => fromBase64Url(bad)).toThrow();
  });
});
