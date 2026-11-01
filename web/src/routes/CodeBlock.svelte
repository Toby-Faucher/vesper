<script lang="ts" module>
  import { lazy } from '../lib/ui/lazy';
  // Its own chunk, fetched the first time a block with a known language is shown.
  const loadHighlight = lazy(() => import('../lib/highlight'));
</script>

<script lang="ts">
  // A fenced code block: the language as a label, a Copy button, and colours once the highlighter has loaded. Until
  // then, or for an unknown language, the code shows plain. Tokens are text nodes in spans: no string is read as HTML.
  import { langOf } from '../lib/markdown';
  import type { Tok } from '../lib/highlight';
  import { copyText } from '../lib/ui/clipboard';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import * as I from '../lib/ui/icons';
  import Icon from '../lib/ui/c/Icon.svelte';

  let { v, lang }: { v: string; lang?: string } = $props();
  let toks = $state<Tok[] | null>(null);
  let copied = $state<'' | 'Copied' | 'failed'>('');

  $effect(() => {
    const l = langOf(lang);
    const code = v;
    toks = null;
    if (!l) return;
    let live = true;
    loadHighlight().then((h) => live && (toks = h.highlight(code, l)), () => {}); // offline: the code stays plain
    return () => (live = false);
  });

  let timer: ReturnType<typeof setTimeout> | undefined;
  async function copy() {
    copied = (await copyText(v)) ? 'Copied' : 'failed';
    clearTimeout(timer);
    timer = setTimeout(() => (copied = ''), 2000);
  }
  $effect(() => () => clearTimeout(timer));
</script>

<div class="code">
  <div class="top">
    <span class="lang">{lang ?? ''}</span>
    <button type="button" class="cp" aria-label={copied === 'Copied' ? 'Copied' : 'Copy code'} onclick={copy}>
      <Icon d={copied === 'Copied' ? I.check : I.copy} size={16} />{copied === 'Copied' ? 'Copied' : 'Copy'}
    </button>
  </div>
  <pre><code>{#if toks}{#each toks as t}{#if t.k}<span class={t.k}>{t.v}</span>{:else}{t.v}{/if}{/each}{:else}{v}{/if}</code></pre>
  <span class="sr-only" role="status">{copied === 'failed' ? UI_MESSAGES.copyFailed : copied}</span>
</div>

<style>
  .code { margin: 4px 0; border: 1px solid var(--line-subtle); border-radius: var(--radius); background: var(--bg-element); white-space: normal; }
  .top { display: flex; align-items: center; justify-content: space-between; height: 28px; padding: 0 4px 0 12px; border-bottom: 1px solid var(--line-subtle); }
  .lang { color: var(--fg-3); font-family: var(--font-mono); font-size: 12px; line-height: 16px; }
  .cp {
    display: inline-flex; align-items: center; gap: 4px; height: 24px; padding: 0 6px; border-radius: var(--radius-sm);
    color: var(--fg-2); font-size: 12px; line-height: 16px; cursor: pointer;
    transition: background-color var(--dur-1) var(--ease-out), color var(--dur-1) var(--ease-out);
  }
  .cp:hover { background: var(--bg-hover); color: var(--fg); }
  pre { overflow-x: auto; padding: 8px 12px; font-family: var(--font-mono); font-size: 13px; line-height: 20px; white-space: pre; }
  .kw { color: var(--syn-kw); }
  .str { color: var(--syn-str); }
  .num { color: var(--syn-num); }
  .ty { color: var(--syn-ty); }
  .com { color: var(--syn-com); font-style: italic; }
  @media (prefers-reduced-motion: reduce) {
    .cp { transition: none; }
  }
</style>
