<script lang="ts">
  import * as I from '../lib/ui/icons';
  import type { Snippet } from 'svelte';
  import type { HTMLInputAttributes } from 'svelte/elements';
  import Icon from '../lib/ui/c/Icon.svelte';
  // The switcher's text field. In the first load, so the dialog opens at once and takes the typing while its chunk loads.
  let { value = $bindable(''), el = $bindable(), lead, placeholder = 'Rooms, people and actions', ...rest }:
    { value?: string; el?: HTMLInputElement; lead?: Snippet } & HTMLInputAttributes = $props();
</script>

<div class="field flex h-12 shrink-0 items-center gap-3 border-b border-subtle px-4 text-fg-3">
  <Icon d={I.search} size={20} />
  {@render lead?.()}
  <input bind:this={el} bind:value class="h-full min-w-0 flex-1 bg-transparent text-lg text-fg placeholder:text-fg-3"
    {placeholder} aria-label="Switch to" autocomplete="off" spellcheck="false" {...rest} />
</div>

<style>
  /* The field always has focus while the dialog is open: the caret and an amber line under it show it, not a ring. */
  .field input:focus-visible { outline: none; }
  .field:focus-within { border-bottom-color: var(--accent-line); }
</style>
