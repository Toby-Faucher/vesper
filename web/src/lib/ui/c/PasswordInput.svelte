<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';
  import { fieldCtx } from './Field.svelte';
  // A password box with a Show toggle. The toggle keeps its name ("Show password") and visible text ("Show"), and
  // aria-pressed carries the state. Every submit of the form hides the password again, so a retry is never typed in clear.
  let { value = $bindable(''), ...rest }: { value?: string } & Omit<HTMLInputAttributes, 'value' | 'type'> = $props();
  const f = fieldCtx();
  let shown = $state(false);
  let input = $state<HTMLInputElement>();
  $effect(() => {
    const form = input?.form;
    if (!form) return;
    const hide = () => (shown = false);
    form.addEventListener('submit', hide);
    return () => form.removeEventListener('submit', hide);
  });
</script>

<div class="relative">
  <input
    bind:this={input}
    id={f?.id}
    aria-describedby={f?.describedby}
    aria-invalid={f?.invalid || undefined}
    {...rest}
    type={shown ? 'text' : 'password'}
    bind:value
    class="control h-8 w-full pr-16 pl-2.5"
  />
  <button type="button" class="show" aria-label="Show password" aria-pressed={shown} aria-controls={f?.id} onclick={() => (shown = !shown)}>Show</button>
</div>

<style>
  .show {
    position: absolute;
    top: 4px;
    right: 4px;
    height: 24px;
    padding: 0 8px;
    border-radius: var(--radius-sm);
    color: var(--fg-2);
    font-size: 12px;
    font-weight: 500;
  }
  .show:hover { background: var(--bg-hover); color: var(--fg); }
  .show[aria-pressed='true'] { background: var(--bg-selected); color: var(--fg); }
</style>
