<script lang="ts">
  // The room's pinned messages, newest pin first, from the header's Pins button. Fetched each time it opens. A row jumps
  // to its message; Unpin is there for everyone, since anyone who sees the room may pin. Text is untrusted: text nodes.
  import { errorText } from '../lib/api';
  import { chat, jumpTo, pin, pins } from '../lib/chat.svelte';
  import type { Message } from '../lib/gen/Message';
  import { parse } from '../lib/markdown';
  import { session } from '../lib/session.svelte';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import { plain } from '../lib/ui/timeline';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Popover from '../lib/ui/c/Popover.svelte';

  let { roomId, anchor, open, onclose }: { roomId: number; anchor: HTMLElement | null; open: boolean; onclose: () => void } = $props();

  const MAX_PINS = 50; // the server's limit per room
  let list = $state<Message[] | null>(null);
  let error = $state('');
  let seq = 0;

  const user = (id: string) => session.state.users[id] ?? { user_id: id, display_name: 'Someone', accent: '' };
  const when = (ms: number) => new Date(ms).toLocaleString([], { day: 'numeric', month: 'short', hour: 'numeric', minute: '2-digit' });

  $effect(() => {
    if (open) void load();
  });

  async function load() {
    const n = ++seq;
    error = '';
    list = null;
    try {
      const got = await pins(roomId);
      if (n === seq) list = got;
    } catch (e) {
      if (n === seq) error = `${UI_MESSAGES.pinsFailed} ${errorText(e)}`;
    }
  }

  async function jump(m: Message) {
    onclose();
    chat.jump = m.id; // the timeline scrolls to it and flashes it once the page lands
    try {
      await jumpTo(roomId, m.id);
    } catch {
      chat.jump = null;
    }
  }

  async function unpin(m: Message) {
    error = '';
    try {
      await pin(m.id, false);
      list = list?.filter((x) => x.id !== m.id) ?? null;
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<Popover {anchor} {open} {onclose} label="Pinned messages">
  <div class="w-78">
    <p class="flex h-8 items-center px-2 font-medium text-fg">Pinned messages</p>
    {#if error}
      <div class="flex items-center gap-2 px-2 pb-2">
        <p class="flex-1 text-xs text-danger" role="alert">{error}</p>
        {#if !list}<Button size="sm" onclick={load}>Try again</Button>{/if}
      </div>
    {/if}
    {#if list === null}
      {#if !error}<p class="px-2 pb-2 text-fg-2" role="status">Loading...</p>{/if}
    {:else if !list.length}
      <div class="px-2 pb-3">
        <p class="text-fg">Nothing is pinned yet.</p>
        <p class="text-xs text-fg-2">Pin a message from its toolbar to keep it here. A room holds up to {MAX_PINS}.</p>
      </div>
    {:else}
      <ul class="max-h-96 overflow-y-auto">
        {#each list as m (m.id)}
          {@const a = user(m.author_id)}
          <li class="pin">
            <Avatar user={a} size={20} />
            <button type="button" class="jump" aria-label="Jump to the message from {a.display_name}: {plain(parse(m.body))}" onclick={() => jump(m)}>
              <span class="flex items-baseline gap-2">
                <span class="truncate font-medium text-fg">{a.display_name}</span>
                <time class="shrink-0 text-xs text-fg-2 tabular-nums">{when(m.created_at)}</time>
              </span>
              <span class="block truncate text-fg-2">{plain(parse(m.body)) || 'A file'}</span>
            </button>
            <Button size="sm" variant="ghost" onclick={() => unpin(m)}>Unpin</Button>
          </li>
        {/each}
      </ul>
      <p class="px-2 pt-1 pb-1 text-xs text-fg-2">{list.length} of {MAX_PINS} pins</p>
    {/if}
  </div>
</Popover>

<style>
  .pin {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) auto;
    align-items: start;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius);
  }
  .pin:hover, .pin:focus-within { background: var(--bg-hover); }
  .pin :global(.av) { margin-top: 0; }
  .jump { min-width: 0; text-align: left; line-height: 20px; }
  .jump:focus-visible { outline-offset: 2px; }
</style>
