<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  // Primary is the amber fill: at most one per view. The rest of the attributes (aria-*, data-*) pass through.
  let {
    variant = 'secondary',
    size = 'md',
    type = 'button',
    el = $bindable(null),
    children,
    ...rest
  }: {
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
    size?: 'sm' | 'md';
    el?: HTMLButtonElement | null; // bind:el to anchor a Popover or Menu
    children: Snippet;
  } & HTMLButtonAttributes = $props();
</script>

<button bind:this={el} {type} {...rest} class="btn {variant} {size}">{@render children()}</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 1px solid transparent;
    border-radius: var(--radius);
    font-weight: 500;
    white-space: nowrap;
    user-select: none;
    transition: background-color var(--dur-1) var(--ease-out), border-color var(--dur-1) var(--ease-out), color var(--dur-1) var(--ease-out);
  }
  .md { height: 32px; padding: 0 12px; }
  .sm { height: 28px; padding: 0 8px; }
  .primary { background: var(--accent); color: var(--on-accent); }
  .primary:hover:enabled { background: var(--accent-hover); }
  .secondary { background: var(--bg-element); border-color: var(--line); color: var(--fg); }
  .secondary:hover:enabled { background: var(--bg-hover); border-color: var(--line-strong); }
  .ghost { color: var(--fg-2); }
  .ghost:hover:enabled { background: var(--bg-hover); color: var(--fg); }
  .danger { background: var(--danger-solid); color: var(--on-danger); }
  .danger:hover:enabled { background: color-mix(in oklab, var(--danger-solid), var(--fg) 8%); }
  .btn:disabled { cursor: not-allowed; background: var(--bg-element); border-color: var(--line-subtle); color: var(--fg-disabled); }
  .ghost:disabled { background: none; border-color: transparent; }
</style>
