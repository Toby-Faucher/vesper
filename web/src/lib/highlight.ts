// A small tokenizer for code blocks, loaded only when a block with a known language is shown (langOf in markdown.ts).
// One sticky-free global regex per language, scanned once left to right: linear in the code. Tokens are rendered as
// text nodes in spans, never as HTML. Coarse on purpose: comments, strings, numbers, keywords, types and tags.

export type Tok = { v: string; k: '' | 'kw' | 'str' | 'com' | 'num' | 'ty' };

const words = (s: string) => new Set(s.split(' '));
const STR2 = `"(?:[^"\\\\\\n]|\\\\.)*"?`; // a string to its close, or to the end of the line when unclosed
const STR1 = `'(?:[^'\\\\\\n]|\\\\.)*'?`;
const NUM = '\\b\\d[\\w.]*';
const BLOCK = '/\\*[\\s\\S]*?(?:\\*/|$)';
const ID = '[A-Za-z_$][\\w$]*';

type Spec = { re: RegExp; kw: Set<string>; types?: boolean };
const spec = (groups: Record<string, string>, kw: string, types = false): Spec => ({
  re: new RegExp(Object.entries(groups).map(([k, v]) => `(?<${k}>${v})`).join('|'), 'g'),
  kw: words(kw),
  types,
});

const LANGS: Record<string, Spec> = {
  rust: spec(
    { com: `//[^\\n]*|${BLOCK}`, str: `${STR2}|'(?:\\\\.|[^\\\\'\\n])'`, num: NUM, ty: '\\b[a-z_]\\w*!(?!=)', w: ID },
    'as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while',
    true,
  ),
  js: spec(
    { com: `//[^\\n]*|${BLOCK}`, str: `${STR2}|${STR1}|\`(?:[^\`\\\\]|\\\\.)*\`?`, num: NUM, w: ID },
    'as async await break case catch class const continue debugger default delete do else enum export extends false finally for from function if import in instanceof interface let new null of return static super switch this throw true try type typeof undefined var void while yield',
    true,
  ),
  json: spec({ ty: `${STR2}(?=\\s*:)`, str: STR2, num: '-?\\d[\\d.eE+-]*', w: ID }, 'true false null'),
  sh: spec(
    { com: '(?<![^\\s;|&])#[^\\n]*', str: `${STR2}|'[^']*'?`, ty: '\\$\\{?[\\w@#?*!-]+\\}?', num: NUM, w: '[A-Za-z_][\\w-]*' },
    'if then else elif fi for in do done case esac while until function return local export readonly set unset exit echo cd sudo',
  ),
  py: spec(
    { com: '#[^\\n]*', str: `[rbfu]{0,2}(?:"""[\\s\\S]*?(?:"""|$)|'''[\\s\\S]*?(?:'''|$)|${STR2}|${STR1})`, ty: '@[\\w.]+', num: NUM, w: ID },
    'and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return self True try while with yield',
    true,
  ),
  html: spec(
    { com: '<!--[\\s\\S]*?(?:-->|$)', kw: '</?[\\w:-]+|/?>', ty: '(?<![\\w:-])[\\w:-]+(?==)', str: `${STR2}|${STR1}` },
    '',
  ),
  css: spec(
    { com: BLOCK, str: `${STR2}|${STR1}`, ty: '(?<![\\w-])[\\w-]+(?=\\s*:[^:])', kw: '@[\\w-]+|!important', num: '#[\\da-fA-F]{3,8}\\b|-?\\d[\\w.%]*' },
    '',
  ),
};

/** `code` as tokens that join back to exactly `code`. `lang` is a name from langOf; an unknown one gives one plain token. */
export function highlight(code: string, lang: string): Tok[] {
  const s = Object.hasOwn(LANGS, lang) ? LANGS[lang] : null;
  if (!s) return [{ v: code, k: '' }];
  const out: Tok[] = [];
  const push = (v: string, k: Tok['k']) => {
    const last = out[out.length - 1];
    if (last?.k === k) last.v += v;
    else if (v) out.push({ v, k });
  };
  const re = new RegExp(s.re); // own lastIndex
  let at = 0;
  for (let m = re.exec(code); m; m = re.exec(code)) {
    if (!m[0]) {
      re.lastIndex++; // no group matches empty today; this keeps a later edit from looping forever
      continue;
    }
    push(code.slice(at, m.index), '');
    const g = m.groups!;
    const k = (Object.keys(g).find((n) => g[n] !== undefined) ?? '') as Tok['k'] | 'w';
    if (k === 'w') push(m[0], s.kw.has(m[0]) ? 'kw' : s.types && /^[A-Z][a-z]/.test(m[0]) ? 'ty' : '');
    else push(m[0], k);
    at = re.lastIndex;
  }
  push(code.slice(at), '');
  return out;
}
