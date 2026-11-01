// A small Markdown subset parsed into a tree that Markdown.svelte renders as text nodes and elements. No HTML string
// is ever built, so there is nothing to sanitise. One left-to-right pass per line with a delimiter stack: linear time.

export type Md =
  | { t: 'text'; v: string }
  | { t: 'br' }
  | { t: 'b' | 'i' | 's' | 'quote'; c: Md[] }
  | { t: 'code'; v: string }
  | { t: 'pre'; v: string }
  | { t: 'link'; href: string }
  | { t: 'mention'; username: string };

const MAX_DEPTH = 4;
const KIND = { '**': 'b', '*': 'i', _: 'i', '~~': 's' } as const;
type Marker = keyof typeof KIND;
type Frame = { marker: Marker; c: Md[] };

const ws = (ch: string | undefined) => ch === undefined || /\s/.test(ch);
const word = (ch: string | undefined) => ch !== undefined && /[\p{L}\p{N}_]/u.test(ch);
const nameChar = (ch: string | undefined) => ch !== undefined && /[A-Za-z0-9_]/.test(ch);

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
      let j = i;
      while (j < src.length && !ws(src[j])) j++;
      while (j > i && '.,;:!?)*_~'.includes(src[j - 1])) j--;
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

/** Parses a message body. Unknown syntax, unclosed markers and raw HTML stay text. */
export function parse(src: string): Md[] {
  const lines = src.split('\n');
  const out: Md[] = [];
  let prevInline = false;
  for (let k = 0; k < lines.length; k++) {
    const line = lines[k];
    if (line.startsWith('```')) {
      // An unclosed fence means no later line starts with ``` either, so this scan runs once at most.
      let e = k + 1;
      while (e < lines.length && !lines[e].startsWith('```')) e++;
      if (e < lines.length) {
        out.push({ t: 'pre', v: lines.slice(k + 1, e).join('\n') });
        k = e;
        prevInline = false;
        continue;
      }
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
    if (prevInline) out.push({ t: 'br' });
    pushAll(out, inline(line));
    prevInline = true;
  }
  return out;
}
