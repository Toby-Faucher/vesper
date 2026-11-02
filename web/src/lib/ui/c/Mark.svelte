<script lang="ts">
  import logo from '../logo.webp';
  // The Vesper logo (assets/logo.png cut to the wordmark, 112 px tall), `size` px tall. Vite hashes the file, so the
  // server sends it as immutable. It reads "Vesper", so it is the name, not decoration. --logo-filter recolours it to the
  // accent (accent.ts).
  let { size = 24 }: { size?: number } = $props();
</script>

<img class="shrink-0" src={logo} alt="Vesper" height={size} width={Math.round((size * 235) / 112)} draggable="false" style="filter: var(--logo-filter)" />
