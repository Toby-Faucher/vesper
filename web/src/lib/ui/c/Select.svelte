<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLSelectAttributes } from 'svelte/elements';
  import { fieldCtx } from './Field.svelte';
  import Icon from './Icon.svelte';
  // Native select (the open list is the browser's, dark from color-scheme); children are the <option>s.
  let { value = $bindable(), children, ...rest }: { value?: unknown; children: Snippet } & Omit<HTMLSelectAttributes, 'value'> = $props();
  const f = fieldCtx();
</script>

<div class="relative">
  <select
    id={f?.id}
    aria-describedby={f?.describedby}
    aria-invalid={f?.invalid || undefined}
    {...rest}
    bind:value
    class="control h-8 w-full appearance-none pr-8 pl-2.5">{@render children()}</select>
  <span class="pointer-events-none absolute inset-y-0 right-2 flex items-center text-fg-3"><Icon name="chevron-down" /></span>
</div>
