<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';
  import { fieldCtx } from './Field.svelte';
  let { value = $bindable(''), ...rest }: { value?: string } & Omit<HTMLInputAttributes, 'value'> = $props();
  const f = fieldCtx();
</script>

<input
  type="text"
  id={f?.id}
  aria-describedby={f?.describedby}
  aria-invalid={f?.invalid || undefined}
  {...rest}
  bind:value
  class="control h-8 w-full px-2.5"
/>
