import { describe, expect, it } from 'vitest';
import { parse, safeUrl, type Md } from './markdown';

const text = (v: string): Md => ({ t: 'text', v });
const links = (nodes: Md[]): string[] =>
  nodes.flatMap((n) => (n.t === 'link' ? [n.href] : 'c' in n ? links(n.c) : []));
const timed = (f: () => void) => {
  const t = performance.now();
  f();
  return performance.now() - t;
};

describe('markdown', () => {
  it('markdown_is_inert', () => {
    expect(parse('<script>alert(1)</script>')).toEqual([text('<script>alert(1)</script>')]);
    expect(parse('<img src=x onerror=alert(1)>')).toEqual([text('<img src=x onerror=alert(1)>')]);
    expect(links(parse('javascript:alert(1)'))).toEqual([]);
    expect(parse('https://x.test/a?b=<c>')).toEqual([{ t: 'link', href: new URL('https://x.test/a?b=<c>').href }]);
    expect(parse('https://x.test/a?b=<c>')).toEqual([{ t: 'link', href: 'https://x.test/a?b=%3Cc%3E' }]);
  });

  it('only http and https links without credentials are links', () => {
    for (const s of ['javascript:alert(1)', 'data:text/html,x', 'vbscript:x', '//evil.test/x', 'https://user:pw@evil.test/', 'https://user@evil.test/', 'ftp://x.test/'])
      expect(safeUrl(s), s).toBeNull();
    for (const s of ['jAvAsCrIpT:alert(1)', 'xhttps://a.test', '[a](javascript:alert(1))', 'https://user:pw@evil.test/'])
      expect(links(parse(s)), s).toEqual([]);
    expect(safeUrl('https://a.test/x')).toBe('https://a.test/x');
    // No [text](url) links: the URL itself shows, so the text cannot hide where it goes.
    expect(parse('[bank](https://evil.test)')).toEqual([text('[bank]('), { t: 'link', href: 'https://evil.test/' }, text(')')]);
  });

  it('links show the real host: bidi and zero-width characters are encoded, not hidden', () => {
    const [l] = parse('https://bank.test/‮gnp.exe');
    expect(l).toEqual({ t: 'link', href: 'https://bank.test/%E2%80%AEgnp.exe' });
    const idn = links(parse('https://аpple.test/'));
    expect(idn).toEqual(['https://xn--pple-43d.test/']);
    for (const h of [...links(parse('https://a.test/x​y')), ...idn]) expect(h).toMatch(/^[\x21-\x7e]+$/);
  });

  it('markdown_basic', () => {
    expect(parse('**b** *i* _j_ ~~s~~ `c<d>`')).toEqual([
      { t: 'b', c: [text('b')] }, text(' '), { t: 'i', c: [text('i')] }, text(' '), { t: 'i', c: [text('j')] }, text(' '),
      { t: 's', c: [text('s')] }, text(' '), { t: 'code', v: 'c<d>' },
    ]);
    expect(parse('a\n```js\n<b>**x**</b>\n```\nb')).toEqual([text('a'), { t: 'pre', v: '<b>**x**</b>' }, text('b')]);
    expect(parse('> one **two**\n> > three\nafter')).toEqual([
      { t: 'quote', c: [text('one '), { t: 'b', c: [text('two')] }, { t: 'br' }, text('> three')] }, text('after'),
    ]);
    expect(parse('hi @Bob_1, @ab @@x a@bcd')).toEqual([
      text('hi '), { t: 'mention', username: 'Bob_1' }, text(', @ab @@x a@bcd'),
    ]);
    expect(parse('see https://a.test/x?y=1). ok')).toEqual([
      text('see '), { t: 'link', href: 'https://a.test/x?y=1' }, text('). ok'),
    ]);
    expect(parse('a\nb\n\nc')).toEqual([text('a'), { t: 'br' }, text('b'), { t: 'br' }, { t: 'br' }, text('c')]);
    expect(parse('**a *b* c**')).toEqual([{ t: 'b', c: [text('a '), { t: 'i', c: [text('b')] }, text(' c')] }]);
    expect(parse('snake_case_name')).toEqual([text('snake_case_name')]);
  });

  it('markdown_unclosed_is_text', () => {
    expect(parse('**a')).toEqual([text('**a')]);
    expect(parse('a **b *c')).toEqual([text('a **b *c')]);
    expect(parse('`a')).toEqual([text('`a')]);
    expect(parse('```\nx')).toEqual([text('```'), { t: 'br' }, text('x')]);
    expect(parse('2 * 3 * 4')).toEqual([text('2 * 3 * 4')]);
    expect(parse('****')).toEqual([text('****')]);
    expect(parse('>no space')).toEqual([text('>no space')]);
  });

  it('nesting stops at depth 4', () => {
    const depth = (n: Md[]): number => Math.max(0, ...n.map((x) => ('c' in x ? 1 + depth(x.c) : 0)));
    expect(depth(parse('*a *b *c *d *e e* d* c* b* a*'))).toBe(4);
  });

  it('markdown_is_linear', () => {
    for (const s of ['*'.repeat(10000), '**a'.repeat(3000)]) expect(timed(() => parse(s))).toBeLessThan(50);
    const big = 100_000;
    for (const s of [
      '*'.repeat(big), '_'.repeat(big), '~'.repeat(big), '`'.repeat(big), '**a'.repeat(big / 3), '*a '.repeat(big / 3),
      '> '.repeat(big / 2), '>\n'.repeat(big / 2), '```\n'.repeat(big / 4), 'https://'.repeat(big / 8), 'http://a.test/'.repeat(big / 14),
      '@'.repeat(big), '@abc'.repeat(big / 4), '<script>'.repeat(big / 8), '**a *b ~~c _d '.repeat(big / 14), 'a'.repeat(big),
    ]) {
      expect(timed(() => parse(s)), s.slice(0, 16)).toBeLessThan(250);
      expect(parse(s).length).toBeGreaterThan(0);
    }
  });
});
