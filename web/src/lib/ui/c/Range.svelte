<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';
  import { fieldCtx } from './Field.svelte';
  // Native range: amber fill up to the thumb, the rest of the track in --bg-selected.
  let {
    value = $bindable(0),
    min = 0,
    max = 100,
    step = 1,
    ...rest
  }: { value?: number; min?: number; max?: number; step?: number } & Omit<HTMLInputAttributes, 'value' | 'min' | 'max' | 'step' | 'type'> = $props();
  const f = fieldCtx();
  const pct = $derived(max > min ? ((value - min) / (max - min)) * 100 : 0);
</script>

<input
  type="range"
  id={f?.id}
  aria-describedby={f?.describedby}
  {min}
  {max}
  {step}
  {...rest}
  bind:value
  style:--pct="{pct}%"
/>

<style>
  input {
    width: 100%;
    height: 16px;
    margin: 0;
    background: none;
    appearance: none;
    accent-color: var(--accent);
    cursor: pointer;
  }
  input:disabled { cursor: not-allowed; opacity: 0.5; }
  input::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--accent) var(--pct), var(--bg-selected) var(--pct));
  }
  input::-moz-range-track { height: 4px; border-radius: 2px; background: var(--bg-selected); }
  input::-moz-range-progress { height: 4px; border-radius: 2px; background: var(--accent); }
  input::-webkit-slider-thumb {
    width: 14px;
    height: 14px;
    margin-top: -5px;
    border-radius: 50%;
    background: var(--fg);
    appearance: none;
  }
  input::-moz-range-thumb { width: 14px; height: 14px; border: 0; border-radius: 50%; background: var(--fg); }
</style>
