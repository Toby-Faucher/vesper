<script lang="ts">
  // The Vesper mark: an amber disc with a four-point sparkle (same drawing as public/favicon.svg).
  let { size = 20 }: { size?: number } = $props();
</script>

<svg class="shrink-0" width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
  <circle cx="16" cy="16" r="15" fill="var(--accent)" />
  <path d="M16 6c.6 5.4 4.6 9.4 10 10-5.4.6-9.4 4.6-10 10-.6-5.4-4.6-9.4-10-10 5.4-.6 9.4-4.6 10-10z" fill="var(--on-accent)" />
</svg>
