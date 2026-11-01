<script lang="ts">
  // Renders a parsed message with text interpolation and fixed elements only: no string is ever read as HTML.
  import type { Md } from '../lib/markdown';

  let { nodes }: { nodes: Md[] } = $props();
</script>

<!-- dir="ltr" isolates each link from bidi characters in the text around it; the shown text is the full href. -->
{#snippet tree(list: Md[])}{#each list as n}{#if n.t === 'text'}{n.v}{:else if n.t === 'br'}<br />{:else if n.t === 'b'}<strong>{@render tree(n.c)}</strong>{:else if n.t === 'i'}<em>{@render tree(n.c)}</em>{:else if n.t === 's'}<s>{@render tree(n.c)}</s>{:else if n.t === 'quote'}<blockquote class="border-l-2 border-line pl-2 text-fg-2">{@render tree(n.c)}</blockquote>{:else if n.t === 'code'}<code class="rounded bg-hover px-1">{n.v}</code>{:else if n.t === 'pre'}<pre class="overflow-x-auto rounded bg-hover p-2"><code>{n.v}</code></pre>{:else if n.t === 'link'}<a href={n.href} rel="noopener noreferrer nofollow" target="_blank" dir="ltr" class="break-all text-fg underline">{n.href}</a>{:else if n.t === 'mention'}<span class="rounded bg-accent-subtle px-0.5 text-accent-fg">@{n.username}</span>{/if}{/each}{/snippet}

<div class="break-words whitespace-pre-wrap">{@render tree(nodes)}</div>
