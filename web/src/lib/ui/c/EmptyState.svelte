<script lang="ts">
  import type { Snippet } from 'svelte';
  // What is missing and the one next step: "Nobody is in voice." + "Join to start." + an optional action.
  let { title, body, action }: { title: string; body?: string; action?: Snippet } = $props();
</script>

<div class="flex flex-col items-center gap-1 px-6 py-12 text-center">
  <p class="text-base font-medium text-fg">{title}</p>
  {#if body}<p class="max-w-80 text-sm text-fg-3">{body}</p>{/if}
  {#if action}<div class="mt-3">{@render action()}</div>{/if}
</div>
