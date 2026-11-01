<script lang="ts">
  // Renders a parsed message with text interpolation and fixed elements only: no string is ever read as HTML.
  // `me` is the reader's username: a mention of them is drawn in amber, other mentions in grey.
  import type { Md } from '../lib/markdown';

  let { nodes, me = '' }: { nodes: Md[]; me?: string } = $props();
  const mine = (u: string) => !!me && u.toLowerCase() === me.toLowerCase();
</script>

<!-- dir="ltr" isolates each link from bidi characters in the text around it; the shown text is the full href. -->
{#snippet tree(list: Md[])}{#each list as n}{#if n.t === 'text'}{n.v}{:else if n.t === 'br'}<br />{:else if n.t === 'b'}<strong class="font-medium text-fg">{@render tree(n.c)}</strong>{:else if n.t === 'i'}<em>{@render tree(n.c)}</em>{:else if n.t === 's'}<s>{@render tree(n.c)}</s>{:else if n.t === 'quote'}<blockquote class="border-l-2 border-line pl-3 text-fg-2">{@render tree(n.c)}</blockquote>{:else if n.t === 'code'}<code class="rounded-sm bg-element px-1 py-px font-mono text-[13px]">{n.v}</code>{:else if n.t === 'pre'}<pre class="my-1 overflow-x-auto rounded-md border border-subtle bg-element px-3 py-2 font-mono text-[13px]"><code>{n.v}</code></pre>{:else if n.t === 'link'}<a href={n.href} rel="noopener noreferrer nofollow" target="_blank" dir="ltr" class="break-all text-accent-fg underline underline-offset-2">{n.href}</a>{:else if n.t === 'mention'}<span class="rounded-sm px-0.5 font-medium {mine(n.username) ? 'bg-accent-subtle text-accent-fg' : 'bg-selected text-fg'}">@{n.username}</span>{/if}{/each}{/snippet}

<div class="break-words whitespace-pre-wrap">{@render tree(nodes)}</div>
