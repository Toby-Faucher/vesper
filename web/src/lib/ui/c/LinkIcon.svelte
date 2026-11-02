<script lang="ts">
  import { linkDetail, linkLabel, type Link } from '../link';
  // Connection quality after a name: three bars for good, two for fair, one in the warning colour for poor, all faint while
  // measuring. The count of bars carries the state, not only the colour. Static: nothing moves when the grade changes.
  let { link, name }: { link: Link; name: string | null } = $props();
  const BARS = { good: 3, fair: 2, poor: 1, measuring: 0 } as const;
  const label = $derived(linkLabel(name, link));
</script>

<span role="img" aria-label={label} title="{label}. {linkDetail(link)}" class="inline-flex shrink-0 items-center"
  class:text-fg-3={link.grade !== 'poor'} class:text-warn={link.grade === 'poor'} data-link={link.grade}>
  <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
    {#each [1, 2, 3] as n (n)}<rect x={n * 4 - 4} y={10 - n * 3} width="3" height={n * 3 + 1} rx="0.5" opacity={n <= BARS[link.grade] ? 1 : 0.3} />{/each}
  </svg>
</span>
