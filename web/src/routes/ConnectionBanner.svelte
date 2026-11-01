<script lang="ts">
  import { onMount } from 'svelte';
  import { session } from '../lib/session.svelte';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import Button from '../lib/ui/c/Button.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  // The socket's state at the top of the main pane. It takes height only while shown and pushes the page down; it never
  // covers it. The status region stays in the page so a screen reader hears each change; the moderator notice is an alert.
  let wasOnline = $state(false);
  let slow = $state(false); // the first connect shows its line only after a moment, so a quick load does not jump
  let now = $state(Date.now());

  onMount(() => {
    const t = setTimeout(() => (slow = true), 600);
    return () => clearTimeout(t);
  });
  $effect(() => {
    if (session.online) wasOnline = true;
  });
  $effect(() => {
    if (session.retryAt === null) return;
    now = Date.now();
    const t = setInterval(() => (now = Date.now()), 250);
    return () => clearInterval(t);
  });
  const secs = $derived(session.retryAt === null ? 0 : Math.max(1, Math.ceil((session.retryAt - now) / 1000)));
  const kind = $derived(
    session.notice || session.online ? null : session.retryAt !== null ? 'retry' : wasOnline ? 'now' : slow ? 'connecting' : null,
  );
</script>

<div role="status" class="shrink-0">
  {#if kind}
    <div class="bar {kind === 'connecting' ? 'calm' : 'warn'}">
      <span class="mark"><Icon name={kind === 'connecting' ? 'info' : 'warn'} /></span>
      <span class="tabular-nums">
        {kind === 'connecting' ? UI_MESSAGES.connecting : kind === 'retry' ? UI_MESSAGES.reconnecting(secs) : UI_MESSAGES.reconnectingNow}
      </span>
      {#if kind === 'retry'}<Button size="sm" variant="ghost" onclick={() => session.retryNow()}>{UI_MESSAGES.retryNow}</Button>{/if}
    </div>
  {/if}
</div>
{#if session.notice}
  <div role="alert" class="bar warn shrink-0">
    <span class="mark"><Icon name="warn" /></span>
    <span>{session.notice}</span>
    <Button size="sm" variant="secondary" onclick={() => session.connect()}>{UI_MESSAGES.reconnect}</Button>
  </div>
{/if}

<style>
  .bar {
    display: flex;
    min-height: 40px;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 16px;
    border-bottom: 1px solid var(--line-subtle);
    color: var(--fg);
  }
  .calm { background: var(--bg-sidebar); color: var(--fg-2); }
  .calm .mark { color: var(--fg-3); }
  .warn { background: var(--warn-subtle); }
  .warn .mark { color: var(--warn-fg); }
  .mark { display: flex; }
</style>
