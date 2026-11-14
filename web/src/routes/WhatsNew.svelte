<script lang="ts">
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import { RELEASE_NOTES, whatsNew } from '../lib/release-notes';
  // "What's new" after an update: the notes of each version since the one this browser saw last, newest first. The
  // shell mounts it only when the server is newer, and never while a call rings. Nothing shows when no note matches.
  let { seen, current, onclose }: { seen: string; current: string; onclose: () => void } = $props();
  const notes = $derived(whatsNew(seen, current, RELEASE_NOTES));
</script>

<Dialog open={!!notes} title={UI_MESSAGES.whatsNewTitle} {onclose}>
  {#each notes ?? [] as n (n.version)}
    <h3 class="mt-3 mb-1 font-medium text-fg first:mt-0">{UI_MESSAGES.whatsNewVersion(n.version)}</h3>
    <ul class="list-disc space-y-1 pl-5">
      {#each n.items as item, i (i)}<li>{item}</li>{/each}
    </ul>
  {/each}
</Dialog>
