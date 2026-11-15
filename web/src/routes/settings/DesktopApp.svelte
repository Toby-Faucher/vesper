<script lang="ts">
  import Icon from '../../lib/ui/c/Icon.svelte';
  import * as I from '../../lib/ui/icons';
  import { DOWNLOADS, RELEASES_PAGE, desktopOs, type DesktopOs } from '../../lib/desktop';
  // Settings > Desktop app: the download for this system first, the other one under it.
  const mine = desktopOs(navigator.userAgent);
  const order: DesktopOs[] = mine === 'linux' ? ['linux', 'windows'] : ['windows', 'linux'];
</script>

<div class="flex flex-col gap-4 text-sm text-fg-2">
  <p>The desktop app has the same rooms, chat and calls, and a push to talk key that works while a game has focus. Sign in with the address of this server and your account.</p>
  <div class="flex flex-wrap gap-2">
    {#each order as os (os)}
      {@const d = DOWNLOADS[os]}
      <a href={d.url} class="dl {os === (mine ?? 'windows') ? 'primary' : 'secondary'}">
        <Icon d={I.download} />Download for {d.label}
      </a>
    {/each}
  </div>
  <p>Windows: unzip it and run <code>Vesper.exe</code>. Linux: unpack it and run <code>Vesper/vesper-desktop</code>. Not on macOS yet.
    Every version is on the <a class="text-accent-fg underline" href={RELEASES_PAGE}>releases page</a>.</p>
</div>

<style>
  /* Links that look like c/Button (md); a download is a link, not a button. */
  .dl {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 12px;
    border: 1px solid transparent;
    border-radius: var(--radius);
    font-weight: var(--weight-ui);
    white-space: nowrap;
    transition: background-color var(--dur-1) var(--ease-out), border-color var(--dur-1) var(--ease-out);
  }
  .primary { background: var(--accent); color: var(--on-accent); }
  .primary:hover { background: var(--accent-hover); }
  .secondary { background: var(--bg-element); border-color: var(--line); color: var(--fg); }
  .secondary:hover { background: var(--bg-hover); border-color: var(--line-strong); }
</style>
