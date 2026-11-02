import { describe, expect, it } from 'vitest';
import { decorate, langOf, parse, safeUrl, type Md } from './markdown';

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

  it('text on a code fence line is shown', () => {
    // The server counts @bob as a mention, so a reader must see it too.
    expect(parse('```js hi @bob\nx\n```after\nnext')).toEqual([
      text('js hi '), { t: 'mention', username: 'bob' }, { t: 'pre', v: 'x' }, text('after'), { t: 'br' }, text('next'),
    ]);
    // One word is the language: the block's label, not text.
    expect(parse('a\n```js\nx\n```')).toEqual([text('a'), { t: 'pre', v: 'x', lang: 'js' }]);
  });

  it('markdown_basic', () => {
    expect(parse('**b** *i* _j_ ~~s~~ `c<d>`')).toEqual([
      { t: 'b', c: [text('b')] }, text(' '), { t: 'i', c: [text('i')] }, text(' '), { t: 'i', c: [text('j')] }, text(' '),
      { t: 's', c: [text('s')] }, text(' '), { t: 'code', v: 'c<d>' },
    ]);
    expect(parse('a\n```\n<b>**x**</b>\n```\nb')).toEqual([text('a'), { t: 'pre', v: '<b>**x**</b>' }, text('b')]);
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
    expect(parse('a ****')).toEqual([text('a ****')]);
    expect(parse('****')).toEqual([{ t: 'hr' }]); // a line of three or more is a rule, as CommonMark
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
      '#'.repeat(big), '# a\n'.repeat(big / 4), '- '.repeat(big / 2), '- a\n'.repeat(big / 4), '1. a\n- b\n'.repeat(big / 10),
      '-'.repeat(big), '- - '.repeat(big / 4) + 'x', '1'.repeat(big) + '. a', '```js\n'.repeat(big / 6),
      '- a\n    1. b\n'.repeat(big / 12), ' '.repeat(big) + '- a', ' - a\n  - b\n   - c\n    - d\n'.repeat(big / 32),
    ]) {
      expect(timed(() => parse(s)), s.slice(0, 16)).toBeLessThan(250);
      expect(timed(() => decorate(s)), s.slice(0, 16)).toBeLessThan(250);
      expect(parse(s).length).toBeGreaterThan(0);
    }
  });

  it('markdown_blocks', () => {
    expect(parse('# One\n## Two **b**\n#### Four\n#no')).toEqual([
      { t: 'h', n: 1, c: [text('One')] }, { t: 'h', n: 2, c: [text('Two '), { t: 'b', c: [text('b')] }] },
      { t: 'h', n: 3, c: [text('Four')] }, text('#no'),
    ]);
    expect(parse('- a\n* b\n+ c\n3. x\n4) y\nafter')).toEqual([
      { t: 'ul', c: [{ t: 'li', c: [text('a')] }, { t: 'li', c: [text('b')] }, { t: 'li', c: [text('c')] }] },
      { t: 'ol', start: 3, c: [{ t: 'li', c: [text('x')] }, { t: 'li', c: [text('y')] }] }, text('after'),
    ]);
    expect(parse('a\n---\n* * *\nb')).toEqual([text('a'), { t: 'hr' }, { t: 'hr' }, text('b')]);
    expect(parse('- - -')).toEqual([{ t: 'hr' }]); // as CommonMark: dashes and spaces only
    for (const s of ['-a', '- ', '#', '# ', '1.a', '--', '2 * 3']) expect(parse(s), s).toEqual([text(s)]);
    expect(parse('```RusT\nfn a() {}\n```')).toEqual([{ t: 'pre', v: 'fn a() {}', lang: 'RusT' }]);
    expect(langOf('RusT')).toBe('rust');
    expect(langOf('tsx')).toBe('js');
    expect(langOf('cobol')).toBeNull();
    expect(langOf(undefined)).toBeNull();
    expect(langOf('constructor')).toBeNull(); // not an Object.prototype key
  });

  it('indented items nest under the item above', () => {
    const li = (v: string, ...sub: Md[]): Md => ({ t: 'li', c: [text(v), ...sub] });
    expect(parse('- a\n  - b\n    1. c\n  - d\n- e')).toEqual([
      { t: 'ul', c: [li('a', { t: 'ul', c: [li('b', { t: 'ol', start: 1, c: [li('c')] }), li('d')] }), li('e')] },
    ]);
    // A change of kind at one level starts a sibling list there; at the top it starts a new list, as before.
    expect(parse('- a\n  - b\n  1. c\n3. d')).toEqual([
      { t: 'ul', c: [li('a', { t: 'ul', c: [li('b')] }, { t: 'ol', start: 1, c: [li('c')] })] },
      { t: 'ol', start: 3, c: [li('d')] },
    ]);
    // An indented first item is a list; a dedent past every level starts a new one.
    expect(parse('  - a\n- b')).toEqual([{ t: 'ul', c: [li('a')] }, { t: 'ul', c: [li('b')] }]);
    // Nesting stops at depth 4: deeper items stay at the fourth level.
    const deep = parse(['- 1', ' - 2', '  - 3', '   - 4', '    - 5'].join('\n'));
    const levels = (n: Md[]): number => Math.max(0, ...n.map((x) => (x.t === 'ul' ? 1 : 0) + ('c' in x ? levels(x.c) : 0)));
    expect(levels(deep)).toBe(4);
    expect(decorate('- a\n  - b')).toEqual([
      { v: '- ', k: 'mk' }, { v: 'a\n', k: '' }, { v: '  - ', k: 'mk' }, { v: 'b', k: '' },
    ]);
  });

  it('hostile input in blocks stays text', () => {
    expect(parse('# <img src=x onerror=alert(1)>')).toEqual([{ t: 'h', n: 1, c: [text('<img src=x onerror=alert(1)>')] }]);
    expect(parse('- [x](javascript:alert(1))')).toEqual([{ t: 'ul', c: [{ t: 'li', c: [text('[x](javascript:alert(1))')] }] }]);
    expect(parse('```<script>\nx\n```')).toEqual([text('<script>'), { t: 'pre', v: 'x' }]); // not one word: shown as text
    expect(parse('```js"onload=x\n</code>\n```')).toEqual([text('js"onload=x'), { t: 'pre', v: '</code>' }]);
    expect(parse('![img](https://x.test/a.png)')).toEqual([text('![img]('), { t: 'link', href: 'https://x.test/a.png' }, text(')')]);
  });

  it('decorate joins back to the source and marks what parse formats', () => {
    const alphabet = ['*', '**', '_', '~~', '`', '```', '> ', '# ', '- ', '1. ', '---', '\n', ' ', 'a', 'b', '@abc', 'https://x.test/a.', '<', 'é'];
    let seed = 7;
    const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
    for (let n = 0; n < 2000; n++) {
      const s = Array.from({ length: 1 + Math.floor(rnd() * 24) }, () => alphabet[Math.floor(rnd() * alphabet.length)]).join('');
      expect(decorate(s).map((r) => r.v).join(''), JSON.stringify(s)).toBe(s);
    }
    expect(decorate('**b** _i_ `c`')).toEqual([
      { v: '**', k: 'mk' }, { v: 'b', k: 'b' }, { v: '**', k: 'mk' }, { v: ' ', k: '' }, { v: '_', k: 'mk' }, { v: 'i', k: 'i' },
      { v: '_', k: 'mk' }, { v: ' ', k: '' }, { v: '`', k: 'mk' }, { v: 'c', k: 'code' }, { v: '`', k: 'mk' },
    ]);
    expect(decorate('# Hi\n> q *x*\n- li')).toEqual([
      { v: '# ', k: 'mk' }, { v: 'Hi', k: 'h' }, { v: '\n', k: '' }, { v: '> ', k: 'mk' }, { v: 'q ', k: 'q' },
      { v: '*', k: 'q mk' }, { v: 'x', k: 'q i' }, { v: '*', k: 'q mk' }, { v: '\n', k: '' }, { v: '- ', k: 'mk' }, { v: 'li', k: '' },
    ]);
    expect(decorate('```js\nlet a\n```')).toEqual([{ v: '```js', k: 'mk' }, { v: '\nlet a', k: 'pre' }, { v: '\n```', k: 'mk' }]);
    expect(decorate('**open')).toEqual([{ v: '**open', k: '' }]); // unclosed is text, as when sent
    expect(decorate('see https://a.test/x).')).toEqual([{ v: 'see ', k: '' }, { v: 'https://a.test/x', k: 'link' }, { v: ').', k: '' }]);
  });
});
