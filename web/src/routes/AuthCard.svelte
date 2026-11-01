<script lang="ts">
  import type { Snippet } from 'svelte';
  import Mark from '../lib/ui/c/Mark.svelte';
  // The page around sign in, signup and the welcome steps: the mark and the name over one centred card.
  // `aside` sits on the right of the name row (the welcome step count); `below` goes under the card.
  let { wide = false, aside, below, children }: { wide?: boolean; aside?: Snippet; below?: Snippet; children: Snippet } = $props();
</script>

<!-- Anchored near the top third rather than centred, so an error line does not move the card. -->
<main class="flex min-h-screen flex-col items-center px-4 pt-[max(40px,22vh)] pb-10">
  <div class="w-full" class:max-w-90={!wide} class:max-w-110={wide}>
    <div class="mb-5 flex items-center gap-2" class:justify-center={!aside}>
      <Mark size={28} />
      <span class="text-lg font-semibold tracking-tight text-fg">Vesper</span>
      {#if aside}<span class="ml-auto">{@render aside()}</span>{/if}
    </div>
    <div class="rounded-lg border border-subtle bg-raised p-6">
      {@render children()}
    </div>
    {#if below}{@render below()}{/if}
  </div>
</main>
