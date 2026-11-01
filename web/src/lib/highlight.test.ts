import { describe, expect, it } from 'vitest';
import { highlight, type Tok } from './highlight';

const kinds = (t: Tok[]) => t.filter((x) => x.k).map((x) => `${x.k}:${x.v}`);

describe('highlight', () => {
  it('tokens join back to the code in every language', () => {
    const src = 'fn main() { let s = "a\\"b"; // c\n /* d */ 42 `t` \'x\' # e <a href="x"> @d $V """q"""\n';
    for (const l of ['rust', 'js', 'json', 'sh', 'py', 'html', 'css', 'nope', 'constructor'])
      expect(highlight(src, l).map((t) => t.v).join(''), l).toBe(src);
    expect(highlight('x', 'constructor')).toEqual([{ v: 'x', k: '' }]);
  });

  it('finds the main kinds', () => {
    expect(kinds(highlight('let x = "a"; // hi\nconst N = 3;', 'js'))).toEqual(['kw:let', 'str:"a"', 'com:// hi', 'kw:const', 'num:3']);
    expect(kinds(highlight('fn f() -> Option<u8> { println!("x") }', 'rust'))).toEqual(['kw:fn', 'ty:Option', 'ty:println!', 'str:"x"']);
    expect(kinds(highlight('a != b', 'rust'))).toEqual([]);
    expect(kinds(highlight('{"k": [1, true, null]}', 'json'))).toEqual(['ty:"k"', 'num:1', 'kw:true', 'kw:null']);
    expect(kinds(highlight('echo "$HOME" # c\na#b', 'sh'))).toEqual(['kw:echo', 'str:"$HOME"', 'com:# c']);
    expect(kinds(highlight('@dec\ndef f(): return None # c', 'py'))).toEqual(['ty:@dec', 'kw:def', 'kw:return', 'kw:None', 'com:# c']);
    expect(kinds(highlight('<a href="x">t</a><!-- c -->', 'html'))).toEqual(['kw:<a', 'ty:href', 'str:"x"', 'kw:>', 'kw:</a>', 'com:<!-- c -->']);
    expect(kinds(highlight('a { color: #fff; } /* c */', 'css'))).toEqual(['ty:color', 'num:#fff', 'com:/* c */']);
  });

  it('is linear on hostile code', () => {
    const big = 100_000;
    for (const s of ['/*'.repeat(big / 2), '"'.repeat(big), "'''".repeat(big / 3), '<!--'.repeat(big / 4), '\\'.repeat(big), 'a'.repeat(big), '#'.repeat(big)])
      for (const l of ['rust', 'js', 'json', 'sh', 'py', 'html', 'css']) {
        const t = performance.now();
        expect(highlight(s, l).map((x) => x.v).join('').length).toBe(s.length);
        expect(performance.now() - t, `${l} ${s.slice(0, 4)}`).toBeLessThan(250);
      }
  });
});
