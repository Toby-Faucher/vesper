<script lang="ts">
  import type { IconPath } from '../icons';
  // Decorative: the control around it carries the name.
  let { d, size = 16 }: { d: IconPath; size?: 16 | 20 } = $props();
</script>

<svg
  class="shrink-0"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path {d} /></svg>
