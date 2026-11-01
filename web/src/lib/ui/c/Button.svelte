<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  // Primary is the amber fill: at most one per view. The rest of the attributes (aria-*, data-*) pass through.
  // `loading`: a spinner over the hidden label (same width), aria-busy, and clicks do nothing. It stays focusable (not
  // disabled), so keyboard focus is not lost while a send or a save runs.
  let {
    variant = 'secondary',
    size = 'md',
    type = 'button',
    loading = false,
    el = $bindable(null),
    children,
    onclick,
    ...rest
  }: {
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger' | 'danger-ghost';
    size?: 'sm' | 'md';
    loading?: boolean;
    el?: HTMLButtonElement | null; // bind:el to anchor a Popover or Menu
    children: Snippet;
  } & HTMLButtonAttributes = $props();
</script>

<button
  bind:this={el}
  {type}
  {...rest}
  class="btn {variant} {size}"
  aria-busy={loading || undefined}
  aria-disabled={loading || rest['aria-disabled']}
  onclick={(e) => (loading ? e.preventDefault() : onclick?.(e))}
>
  <span class="label">{@render children()}</span>
  {#if loading}
    <svg class="spin" width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <circle cx="8" cy="8" r="6" stroke="currentColor" stroke-opacity="0.3" stroke-width="2" />
      <path d="M14 8a6 6 0 0 0-6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
    </svg>
  {/if}
</button>

<style>
  .btn {
    position: relative;
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
  .danger-ghost { color: var(--danger-fg); }
  .danger-ghost:hover:enabled { background: var(--danger-subtle); }
  .label { display: contents; }
  [aria-busy] { cursor: progress; }
  [aria-busy] .label { visibility: hidden; }
  .spin { position: absolute; inset: 0; margin: auto; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(1turn); } }
  .btn:disabled { cursor: not-allowed; background: var(--bg-element); border-color: var(--line-subtle); color: var(--fg-disabled); }
  .ghost:disabled { background: none; border-color: transparent; }
</style>
