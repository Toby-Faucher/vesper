<script lang="ts" module>
  import { getContext } from 'svelte';
  export type FieldCtx = { id: string; describedby: string | undefined; invalid: boolean };
  // TextInput, Select and Range inside a Field take its id and descriptions from here.
  export const fieldCtx = () => getContext<FieldCtx | undefined>('vesper.field');
</script>

<script lang="ts">
  import { setContext, type Snippet } from 'svelte';
  // Label above the control, then the error (role="alert") or else the help line, tied with aria-describedby.
  // `value`: the control's current value as text, at the right of the label (a range's dB or kbps).
  let { label, help, error, value, children }: { label: string; help?: string; error?: string; value?: string; children: Snippet } = $props();
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
  {#if value === undefined}
    <label for={id} class="text-sm font-medium text-fg">{label}</label>
  {:else}
    <div class="flex items-baseline justify-between gap-4">
      <label for={id} class="text-sm font-medium text-fg">{label}</label>
      <span class="text-sm text-fg-2 tabular-nums" aria-hidden="true">{value}</span>
    </div>
  {/if}
  {@render children()}
  {#if error}
    <p id="{id}-error" role="alert" class="text-xs text-danger">{error}</p>
  {:else if help}
    <p id="{id}-help" class="text-xs text-fg-2">{help}</p>
  {/if}
</div>
