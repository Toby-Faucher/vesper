<script lang="ts">
  import { ICONS, type IconName } from '../icons';
  // Decorative: the control around it carries the name.
  let { name, size = 16 }: { name: IconName; size?: 16 | 20 } = $props();
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
  aria-hidden="true"><path d={ICONS[name]} /></svg>
