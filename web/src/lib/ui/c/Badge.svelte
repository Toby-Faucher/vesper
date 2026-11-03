<script lang="ts">
  // An unread count, or the amber @n pill for mentions. aria-hidden: the row label says it in words. With `pop` (the
  // sidebar) it pops in when it appears or its number changes; no exit, the old number would take room. Not in the switcher,
  // whose rows are rebuilt while you type.
  import { motion } from '../motion';
  let { kind, n, pop = false }: { kind: 'count' | 'mention'; n: number; pop?: boolean } = $props();
  const text = $derived(n > 99 ? '99+' : String(n));
</script>

{#key text}<span in:motion|global={pop ? 'bump' : null}
  class="badge inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full px-1.5 text-xs font-medium tabular-nums {kind === 'mention'
    ? 'bg-accent text-on-accent'
    : 'bg-selected text-fg-2'}"
  aria-hidden="true">{kind === 'mention' ? `@${text}` : text}</span
>{/key}
