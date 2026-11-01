<script lang="ts" module>
  import { getContext } from 'svelte';
  export type FieldCtx = { id: string; describedby: string | undefined; invalid: boolean };
  // TextInput, Select and Range inside a Field take its id and descriptions from here.
  export const fieldCtx = () => getContext<FieldCtx | undefined>('vesper.field');
</script>

<script lang="ts">
  import { setContext, type Snippet } from 'svelte';
  // Label above the control, then the error (role="alert") or else the help line, tied with aria-describedby.
  let { label, help, error, children }: { label: string; help?: string; error?: string; children: Snippet } = $props();
  const id = $props.id();
  setContext<FieldCtx>('vesper.field', {
    id,
    get describedby() {
      return error ? `${id}-error` : help ? `${id}-help` : undefined;
    },
    get invalid() {
      return !!error;
    },
  });
</script>

<div class="flex flex-col gap-1.5">
  <label for={id} class="text-sm font-medium text-fg">{label}</label>
  {@render children()}
  {#if error}
    <p id="{id}-error" role="alert" class="text-xs text-danger">{error}</p>
  {:else if help}
    <p id="{id}-help" class="text-xs text-fg-3">{help}</p>
  {/if}
</div>
