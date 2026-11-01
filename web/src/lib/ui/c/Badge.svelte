<script lang="ts">
  // An unread count, or the amber @n pill for mentions. aria-hidden: the row label says it in words.
  let { kind, n }: { kind: 'count' | 'mention'; n: number } = $props();
  const text = $derived(n > 99 ? '99+' : String(n));
</script>

<span
  class="inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full px-1.5 text-xs font-medium tabular-nums {kind === 'mention'
    ? 'bg-accent text-on-accent'
    : 'bg-selected text-fg-2'}"
  aria-hidden="true">{kind === 'mention' ? `@${text}` : text}</span
>
