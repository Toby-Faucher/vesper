// Test helper: the `--name: value;` pairs of one rule in a css file (rules here do not nest).
export function block(css: string, selector: string): Map<string, string> {
  const at = css.indexOf(selector + ' {');
  if (at < 0) throw new Error(`no rule ${selector}`);
  const body = css.slice(css.indexOf('{', at) + 1, css.indexOf('}', at));
  return new Map([...body.matchAll(/(--[\w-]+)\s*:\s*((?:[^;(]|\([^)]*\))+);/g)].map((m) => [m[1], m[2].trim()]));
}
