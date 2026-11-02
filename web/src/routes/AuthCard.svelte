<script lang="ts">
  import type { Snippet } from 'svelte';
  import Mark from '../lib/ui/c/Mark.svelte';
  import { sidebar } from '../lib/ui/sidebar.svelte';
  // The page around sign in, signup and the welcome steps: the logo over one centred card.
  // `aside` sits on the right of the name row (the welcome step count); `below` goes under the card. `pane`: inside the
  // Shell's <main> (the welcome steps), so a scrolling div rather than a second main landmark. There the sidebar already
  // shows the logo, so the card shows it only while the sidebar is hidden.
  let { wide = false, pane = false, aside, below, children }: { wide?: boolean; pane?: boolean; aside?: Snippet; below?: Snippet; children: Snippet } = $props();
</script>

<!-- Anchored near the top third rather than centred, so an error line does not move the card. -->
<svelte:element this={pane ? 'div' : 'main'} class="flex flex-col items-center px-4 pb-10 {pane ? 'min-h-0 flex-1 overflow-y-auto pt-[max(24px,12vh)]' : 'min-h-screen pt-[max(40px,22vh)]'}">
  <div class="w-full" class:max-w-90={!wide} class:max-w-110={wide}>
    {#if !pane || sidebar.collapsed}
      <div class="flex items-center gap-2" class:mb-5={!pane} class:mb-3={pane} class:justify-center={!aside}>
        <Mark size={pane ? 40 : 56} />
        {#if aside}<span class="ml-auto">{@render aside()}</span>{/if}
      </div>
    {/if}
    <div class="rounded-lg border border-subtle bg-raised p-6">
      {@render children()}
    </div>
    {#if below}{@render below()}{/if}
  </div>
</svelte:element>
