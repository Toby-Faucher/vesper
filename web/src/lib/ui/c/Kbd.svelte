<script lang="ts">
  import type { Snippet } from 'svelte';
  let { children }: { children: Snippet } = $props();
</script>

<kbd
  class="inline-flex h-5 min-w-5 items-center justify-center rounded-sm border border-line border-b-strong bg-element px-1 font-sans text-xs text-fg-2"
  >{@render children()}</kbd>
