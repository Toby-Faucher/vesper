<script lang="ts">
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import ShortcutList from './ShortcutList.svelte';
  let { open, onclose }: { open: boolean; onclose: () => void } = $props();
</script>

<Dialog {open} title="Keyboard shortcuts" {onclose}><ShortcutList /></Dialog>
