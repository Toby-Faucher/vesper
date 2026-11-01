<script lang="ts">
  import Switch from '../../lib/ui/c/Switch.svelte';
  // The two settings mention notifications will bring. Shown off and disabled, with the reason, until they work.
</script>

<div class="flex flex-col gap-2">
  <Switch label="Notify me when someone mentions me" help="Not built yet. Mentions show as a count on the room for now." disabled />
  <Switch label="Play a sound when someone joins or leaves your call" help="Not built yet." disabled />
</div>
