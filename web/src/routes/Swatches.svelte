<script lang="ts">
  import { SLOT_HEX } from '../lib/ui/avatar';
  // The 8 avatar colours as a radio group (Welcome and Settings > Account). `slot` is 1 to 8. `gap`: the background
  // behind the swatches, drawn between a swatch and its ring.
  const COLOURS = ['Coral', 'Orange', 'Lime', 'Green', 'Teal', 'Sky', 'Blue', 'Pink'];
  let { slot = $bindable(1), gap = 'var(--bg-raised)' }: { slot?: number; gap?: string } = $props();
</script>

<fieldset>
  <legend class="mb-2 text-sm font-ui text-fg">Colour</legend>
  <div class="flex gap-2.5" style:--gap={gap}>
    {#each SLOT_HEX as _, i (i)}
      <label class="swatch" style:--c="var(--avatar-{i + 1})">
        <input type="radio" name="colour" class="absolute inset-0 m-0 cursor-pointer appearance-none rounded-full outline-none" aria-label={COLOURS[i]} value={i + 1} bind:group={slot} />
      </label>
    {/each}
  </div>
</fieldset>

<style>
  .swatch {
    position: relative;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--c);
    cursor: pointer;
    box-shadow: 0 0 0 2px var(--gap), 0 0 0 4px transparent;
    transition: box-shadow var(--dur-1) var(--ease-out);
  }
  .swatch:has(:checked) { box-shadow: 0 0 0 2px var(--gap), 0 0 0 4px var(--fg); }
  .swatch:has(:focus-visible) { outline: 2px solid var(--focus); outline-offset: 5px; }
</style>
