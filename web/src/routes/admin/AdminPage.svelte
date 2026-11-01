<script lang="ts">
  import type { AdminSection } from '../../lib/route';
  import { UI_MESSAGES } from '../../lib/ui/messages';
  import EmptyState from '../../lib/ui/c/EmptyState.svelte';
  // A lazy chunk. Tasks 15 and 16 fill the sections.
  let { section }: { section: AdminSection } = $props();
</script>

<div class="flex-1" data-section={section}><EmptyState title={UI_MESSAGES.adminLater} /></div>
