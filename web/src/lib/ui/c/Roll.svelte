<script lang="ts">
  // Number flip: each digit of `text` that changes rolls (roll() in motion.ts), up when the number grows. Columns are
  // keyed from the right, so 9 to 10 keeps the ones column and rolls a new tens column in. aria-hidden: the caller
  // says the number in words.
  import { untrack } from 'svelte';
  import { roll } from '../motion';
  let { text }: { text: string } = $props();
  const value = (t: string) => parseInt(t.replace(/\D/g, ''), 10) || 0;
  // Read by the transitions when they start, so plain values: set before the DOM changes.
  const dir = { last: untrack(() => text), up: true };
  $effect.pre(() => {
    dir.up = value(text) >= value(dir.last);
    dir.last = text;
  });
</script>

<span class="roll" aria-hidden="true">{#each [...text] as ch, i (text.length - i)}<span class="col">{#key ch}<span in:roll|global={{ up: dir.up }} out:roll|global={{ up: dir.up }}>{ch}</span>{/key}</span>{/each}</span>

<style>
  .roll { display: inline-flex; font-variant-numeric: tabular-nums; }
  .col { display: inline-grid; overflow: clip; }
  .col > :global(*) { grid-area: 1 / 1; }
</style>
