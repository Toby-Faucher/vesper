// A small Markdown subset parsed into a tree that Markdown.svelte renders as text nodes and elements. No HTML string
// is ever built, so there is nothing to sanitise. One left-to-right pass per line with a delimiter stack: linear time.
// The subset is written down in docs/architecture.md (Markdown). `decorate` cuts the same source into styled runs for
// the composer, so what you type is styled by the rules that render it.

export type Md =
  | { t: 'text'; v: string }
  | { t: 'br' }
  | { t: 'hr' }
  | { t: 'b' | 'i' | 's' | 'quote' | 'li'; c: Md[] }
  | { t: 'h'; n: 1 | 2 | 3; c: Md[] }
  | { t: 'ul'; c: Md[] }
  | { t: 'ol'; start: number; c: Md[] }
  | { t: 'code'; v: string }
  | { t: 'pre'; v: string; lang?: string }
  | { t: 'link'; href: string }
  | { t: 'mention'; username: string };

const MAX_DEPTH = 4;
const KIND = { '**': 'b', '*': 'i', _: 'i', '~~': 's' } as const;
type Marker = keyof typeof KIND;
type Frame = { marker: Marker; c: Md[] };

const ws = (ch: string | undefined) => ch === undefined || /\s/.test(ch);
const word = (ch: string | undefined) => ch !== undefined && /[\p{L}\p{N}_]/u.test(ch);
const nameChar = (ch: string | undefined) => ch !== undefined && /[A-Za-z0-9_]/.test(ch);

const HEADING = /^(#{1,6}) +(?=\S)/;
const HR = /^ {0,3}([-*_])(?: *\1){2,} *$/;
const ITEM = /^( *)([-*+]|(\d{1,9})[.)]) +(?=\S)/; // leading spaces nest the item
const LANG = /^[A-Za-z0-9+#._-]{1,20}$/; // one word after ``` is the language, shown as a label
// The languages the lazy highlighter knows, by every name people type after ```.
const LANGS: Record<string, string> = {
  rust: 'rust', rs: 'rust', js: 'js', 'javascript': 'js', jsx: 'js', mjs: 'js', ts: 'js', typescript: 'js', tsx: 'js',
  json: 'json', sh: 'sh', bash: 'sh', shell: 'sh', zsh: 'sh', console: 'sh', py: 'py', python: 'py',
  html: 'html', xml: 'html', svg: 'html', svelte: 'html', css: 'css',
};
/** The highlighter's name for a fence language, or null when it has none. */
export function langOf(lang: string | undefined): string | null {
  const l = lang?.toLowerCase() ?? '';
  return Object.hasOwn(LANGS, l) ? LANGS[l] : null;
}

/** The parsed href if `s` is an http or https URL without credentials, else null. */
export function safeUrl(s: string): string | null {
  let u: URL;
  try {
    u = new URL(s);
  } catch {
    return null;
  }
  if ((u.protocol !== 'http:' && u.protocol !== 'https:') || u.username || u.password) return null;
  return u.href; // ASCII only: other characters are percent-encoded and hosts punycoded, so the shown text is the link
}

function pushText(out: Md[], v: string): void {
  if (!v) return;
  const last = out[out.length - 1];
  if (last?.t === 'text') out[out.length - 1] = { t: 'text', v: last.v + v };
  else out.push({ t: 'text', v });
}

function pushAll(out: Md[], nodes: Md[]): void {
  for (const n of nodes) {
    if (n.t === 'text') pushText(out, n.v);
    else out.push(n);
  }
}

/** Where a bare link that starts at `i` ends: at white space, less trailing punctuation. */
function linkEnd(src: string, i: number): number {
  let j = i;
  while (j < src.length && !ws(src[j])) j++;
  while (j > i && '.,;:!?)*_~'.includes(src[j - 1])) j--;
  return j;
}

/** Inline Markdown of one line (no newlines). */
function inline(src: string): Md[] {
  const root: Md[] = [];
  const stack: Frame[] = [];
  const open: Record<Marker, number> = { '**': 0, '*': 0, _: 0, '~~': 0 };
  const top = () => (stack.length ? stack[stack.length - 1].c : root);
  // An unclosed frame becomes its marker as text followed by its content.
  const unwind = () => {
    const f = stack.pop()!;
    open[f.marker]--;
    pushText(top(), f.marker);
    pushAll(top(), f.c);
  };
  let i = 0;
  let start = 0; // start of the pending plain text
  const flush = () => pushText(top(), src.slice(start, i));
  const emit = (n: Md, next: number) => {
    flush();
    top().push(n);
    i = start = next;
  };

  while (i < src.length) {
    const ch = src[i];
    const prev = src[i - 1];
    if (ch === '*' || ch === '_' || ch === '~') {
      const marker: Marker | null =
        ch === '~' ? (src[i + 1] === '~' ? '~~' : null) : ch === '*' && src[i + 1] === '*' ? '**' : ch;
      if (!marker) {
        i++;
        continue;
      }
      const next = src[i + marker.length];
      // `_` inside a word (snake_case) is text.
      if (open[marker] > 0 && !ws(prev) && !(marker === '_' && word(next))) {
        flush();
        while (stack[stack.length - 1].marker !== marker) unwind();
        const f = stack.pop()!;
        open[marker]--;
        if (f.c.length) top().push({ t: KIND[marker], c: f.c });
        else pushText(top(), marker + marker);
        i = start = i + marker.length;
      } else if (!ws(next) && !(marker === '_' && word(prev)) && stack.length < MAX_DEPTH) {
        flush();
        stack.push({ marker, c: [] });
        open[marker]++;
        i = start = i + marker.length;
      } else i += marker.length;
    } else if (ch === '`') {
      // When there is no closing backtick, no later backtick exists either, so this scan runs once per line at most.
      const j = src.indexOf('`', i + 1);
      if (j > i + 1) emit({ t: 'code', v: src.slice(i + 1, j) }, j + 1);
      else i = j === i + 1 ? i + 2 : i + 1;
    } else if (ch === 'h' && !word(prev) && (src.startsWith('http://', i) || src.startsWith('https://', i))) {
      const j = linkEnd(src, i);
      const href = safeUrl(src.slice(i, j));
      if (href) emit({ t: 'link', href }, j);
      else i = j;
    } else if (ch === '@' && !nameChar(prev) && prev !== '@') {
      let j = i + 1;
      while (nameChar(src[j])) j++;
      if (j - i - 1 >= 3 && j - i - 1 <= 24) emit({ t: 'mention', username: src.slice(i + 1, j) }, j);
      else i = j;
    } else i++;
  }
  flush();
  while (stack.length) unwind();
  return root;
}

/** The closing fence line for a fence at line `k`, or -1. An unclosed fence means no later line starts with ``` either,
 * so the scan runs once per message at most. */
function fenceEnd(lines: string[], k: number): number {
  if (!lines[k].startsWith('```')) return -1;
  let e = k + 1;
  while (e < lines.length && !lines[e].startsWith('```')) e++;
  return e < lines.length ? e : -1;
}

/** Parses a message body. Unknown syntax, unclosed markers and raw HTML stay text. */
export function parse(src: string): Md[] {
  const lines = src.split('\n');
  const out: Md[] = [];
  let prevInline = false;
  for (let k = 0; k < lines.length; k++) {
    const line = lines[k];
    const e = fenceEnd(lines, k);
    if (e >= 0) {
      // Text on a fence line (an info string that is not one word, or a tail after the closing fence) is shown, so
      // nothing is hidden. One word is the language, shown as the block's label.
      const [head, tail] = [line.slice(3), lines[e].slice(3)];
      const lang = LANG.test(head) ? head : undefined;
      if (head && !lang) {
        if (prevInline) out.push({ t: 'br' });
        pushAll(out, inline(head));
      }
      const v = lines.slice(k + 1, e).join('\n');
      out.push(lang ? { t: 'pre', v, lang } : { t: 'pre', v });
      pushAll(out, inline(tail));
      k = e;
      prevInline = tail !== '';
      continue;
    }
    if (line.startsWith('> ')) {
      const c: Md[] = [];
      for (; k < lines.length && lines[k].startsWith('> '); k++) {
        if (c.length) c.push({ t: 'br' });
        pushAll(c, inline(lines[k].slice(2)));
      }
      k--;
      out.push({ t: 'quote', c });
      prevInline = false;
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      out.push({ t: 'h', n: Math.min(h[1].length, 3) as 1 | 2 | 3, c: inline(line.slice(h[0].length)) });
      prevInline = false;
      continue;
    }
    if (HR.test(line)) {
      out.push({ t: 'hr' });
      prevInline = false;
      continue;
    }
    const m = ITEM.exec(line);
    if (m) {
      // A run of items is one or more lists. An item indented deeper than the one above opens a list inside it (up to
      // MAX_DEPTH levels); a change of kind (bullets or numbers) at one level starts a sibling list there.
      type Open = { l: Md[]; ind: number; ordered: boolean; into: Md[] };
      const open: Open[] = [];
      let n: RegExpExecArray | null;
      for (; k < lines.length && (n = ITEM.exec(lines[k])) && !HR.test(lines[k]); k++) {
        const ind = n[1].length;
        const ordered = n[3] !== undefined;
        while (open.length && ind < open[open.length - 1].ind) open.pop();
        let top: Open | undefined = open[open.length - 1];
        if (top && top.ordered !== ordered && !(ind > top.ind && open.length < MAX_DEPTH)) {
          open.pop();
          top = { l: [], ind: top.ind, ordered, into: top.into };
        } else if (!top || (ind > top.ind && open.length < MAX_DEPTH)) {
          const into = top ? (top.l[top.l.length - 1] as { c: Md[] }).c : out;
          top = { l: [], ind, ordered, into };
        } else top = undefined; // same list
        if (top) {
          top.into.push(ordered ? { t: 'ol', start: Number(n[3]), c: top.l } : { t: 'ul', c: top.l });
          open.push(top);
        }
        open[open.length - 1].l.push({ t: 'li', c: inline(lines[k].slice(n[0].length)) });
      }
      k--;
      prevInline = false;
      continue;
    }
    if (prevInline) out.push({ t: 'br' });
    pushAll(out, inline(line));
    prevInline = true;
  }
  return out;
}

/** A run of composer text and its style names (space separated, empty for plain). */
export type Run = { v: string; k: string };

/**
 * The source cut into styled runs for the composer's highlight layer, by the same rules as `parse`. The runs join back
 * to exactly `src`, character for character, so the layer lines up with the textarea above it. Markers get `mk`.
 */
export function decorate(src: string): Run[] {
  const lines = src.split('\n');
  const out: Run[] = [];
  const join = (a: string, b: string) => (a ? `${a} ${b}` : b);
  const add = (v: string, k: string) => {
    if (!v) return;
    const last = out[out.length - 1];
    if (last?.k === k) last.v += v;
    else out.push({ v, k });
  };
  // Walks the tree of one line next to its source. Text and code are verbatim, markers are known by kind, a link's
  // length comes from linkEnd, so `pos` always points at the source of the node in hand.
  const line = (s: string, base: string) => {
    let pos = 0;
    const walk = (nodes: Md[], cls: string) => {
      for (const n of nodes) {
        if (n.t === 'text') {
          add(n.v, cls);
          pos += n.v.length;
        } else if (n.t === 'code') {
          add('`', join(cls, 'mk'));
          add(n.v, join(cls, 'code'));
          add('`', join(cls, 'mk'));
          pos += n.v.length + 2;
        } else if (n.t === 'link') {
          const j = linkEnd(s, pos);
          add(s.slice(pos, j), join(cls, 'link'));
          pos = j;
        } else if (n.t === 'mention') {
          add(s.slice(pos, pos + n.username.length + 1), cls);
          pos += n.username.length + 1;
        } else if (n.t === 'b' || n.t === 'i' || n.t === 's') {
          const mk = s.slice(pos, pos + (n.t === 'i' ? 1 : 2));
          add(mk, join(cls, 'mk'));
          pos += mk.length;
          walk(n.c, join(cls, n.t));
          add(mk, join(cls, 'mk'));
          pos += mk.length;
        }
      }
    };
    walk(inline(s), base);
  };
  for (let k = 0; k < lines.length; k++) {
    if (k) add('\n', '');
    const l = lines[k];
    const e = fenceEnd(lines, k);
    if (e >= 0) {
      const head = l.slice(3);
      add('```', 'mk');
      if (LANG.test(head)) add(head, 'mk');
      else line(head, '');
      for (let j = k + 1; j < e; j++) add(`\n${lines[j]}`, 'pre');
      add('\n```', 'mk');
      line(lines[e].slice(3), '');
      k = e;
      continue;
    }
    if (l.startsWith('> ')) {
      add('> ', 'mk');
      line(l.slice(2), 'q');
      continue;
    }
    const h = HEADING.exec(l);
    const m = h || HR.test(l) ? null : ITEM.exec(l);
    if (h) {
      add(h[0], 'mk');
      line(l.slice(h[0].length), 'h');
    } else if (HR.test(l)) add(l, 'mk');
    else if (m) {
      add(m[0], 'mk');
      line(l.slice(m[0].length), '');
    } else line(l, '');
  }
  return out;
}
