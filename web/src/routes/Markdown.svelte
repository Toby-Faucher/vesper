<script lang="ts">
  // Renders a parsed message with text interpolation and fixed elements only: no string is ever read as HTML.
  // `me` is the reader's username: a mention of them is drawn in amber, other mentions in grey. A mention of someone in
  // the user list is a button that opens their card; an unknown name stays text.
  import type { Md } from '../lib/markdown';
  import { session } from '../lib/session.svelte';
  import { byUsername } from '../lib/ui/person.svelte';
  import PersonButton from '../lib/ui/c/PersonButton.svelte';
  import CodeBlock from './CodeBlock.svelte';

  let { nodes, me = '' }: { nodes: Md[]; me?: string } = $props();
  const mine = (u: string) => !!me && u.toLowerCase() === me.toLowerCase();
</script>

<!-- dir="ltr" isolates each link from bidi characters in the text around it; the shown text is the full href. -->
{#snippet tree(list: Md[])}{#each list as n}{#if n.t === 'text'}{n.v}{:else if n.t === 'br'}<br />{:else if n.t === 'b'}<strong class="font-medium text-fg">{@render tree(n.c)}</strong>{:else if n.t === 'i'}<em>{@render tree(n.c)}</em>{:else if n.t === 's'}<s>{@render tree(n.c)}</s>{:else if n.t === 'quote'}<blockquote class="border-l-2 border-line pl-3 text-fg-2">{@render tree(n.c)}</blockquote>{:else if n.t === 'code'}<code class="rounded-sm bg-element px-1 py-px font-mono text-[13px]">{n.v}</code>{:else if n.t === 'pre'}<CodeBlock v={n.v} lang={n.lang} />{:else if n.t === 'h'}<p role="heading" aria-level={n.n + 3} class="h h{n.n}">{@render tree(n.c)}</p>{:else if n.t === 'ul'}<ul class="list-disc">{@render tree(n.c)}</ul>{:else if n.t === 'ol'}<ol class="list-decimal" start={n.start}>{@render tree(n.c)}</ol>{:else if n.t === 'li'}<li>{@render tree(n.c)}</li>{:else if n.t === 'hr'}<hr class="my-2 border-line" />{:else if n.t === 'link'}<a href={n.href} rel="noopener noreferrer nofollow" target="_blank" dir="ltr" class="break-all text-accent-fg underline underline-offset-2">{n.href}</a>{:else if n.t === 'mention'}{@const u = byUsername(session.state.users, n.username)}{@const cls = `rounded-sm px-0.5 font-medium ${mine(n.username) ? 'bg-accent-subtle text-accent-fg' : 'bg-selected text-fg'}`}{#if u}<PersonButton id={u.user_id} name="@{n.username}" class={cls}>@{n.username}</PersonButton>{:else}<span class={cls}>@{n.username}</span>{/if}{/if}{/each}{/snippet}

<div class="md break-words whitespace-pre-wrap">{@render tree(nodes)}</div>

<style>
  /* Headings in a message stay small: 20, 16 and 14 px from the type scale, strong weight. aria-level starts at 4, under the page's own. */
  .md :global(.h) { margin: 4px 0 2px; color: var(--fg); font-weight: var(--weight-strong); }
  .md :global(.h1) { font-size: 20px; line-height: 28px; }
  .md :global(.h2) { font-size: 16px; line-height: 24px; }
  .md :global(.h3) { font-size: 14px; line-height: 20px; }
  .md :global(ul), .md :global(ol) { margin: 2px 0; padding-left: 20px; white-space: normal; }
  .md :global(li) { white-space: pre-wrap; }
  .md :global(li::marker) { color: var(--fg-3); }
  .md :global(li > ul) { list-style-type: circle; }
</style>
