<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';
  // A checkbox with role="switch", drawn as a 28x16 track; the label sits left of it on one row, `help` under the label.
  let {
    checked = $bindable(false),
    label,
    help,
    ...rest
  }: { checked?: boolean; label: string; help?: string } & Omit<HTMLInputAttributes, 'checked' | 'type' | 'role'> = $props();
  const id = $props.id();
</script>

<div>
  <label class="flex min-h-8 items-center justify-between gap-4 text-sm text-fg has-disabled:text-fg-disabled">
    <span>{label}</span>
    <input type="checkbox" role="switch" aria-describedby={help ? `${id}-help` : undefined} {...rest} bind:checked />
  </label>
  {#if help}<p id="{id}-help" class="-mt-1 pr-11 text-xs text-fg-3">{help}</p>{/if}
</div>

<style>
  input {
    position: relative;
    flex-shrink: 0;
    width: 28px;
    height: 16px;
    margin: 0;
    border-radius: 8px;
    background: var(--bg-selected);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    appearance: none;
    cursor: pointer;
    transition: background-color var(--dur-1) var(--ease-out);
  }
  input::before {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--fg-2);
    transition: transform var(--dur-2) var(--ease-out);
  }
  input:checked { background: var(--accent); box-shadow: none; }
  input:checked::before { background: var(--on-accent); transform: translateX(12px); }
  input:disabled { cursor: not-allowed; opacity: 0.5; }
</style>
