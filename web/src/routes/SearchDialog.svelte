<script lang="ts">
  import * as I from '../lib/ui/icons';
  import { tick, untrack } from 'svelte';
  import { href } from '../lib/route';
  import { navigate, session } from '../lib/session.svelte';
  import { chat, jumpTo, marks, search } from '../lib/chat.svelte';
  import { errorText } from '../lib/api';
  import type { SearchHit } from '../lib/gen/SearchHit';
  import { loadUsers } from '../lib/ui/users';
  import { dayLabel } from '../lib/ui/timeline';
  import { MAX_QUERY, complete, keyAt, keyValue, nothingToSearch, parseSearch, setKey, shortLast, type Key } from '../lib/ui/searchq';
  import { Searcher } from '../lib/ui/search.svelte';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import Kbd from '../lib/ui/c/Kbd.svelte';
  import SwitcherField from './SwitcherField.svelte';
  // Ctrl/Cmd+Shift+F, the header's Search button or the switcher: message search over the rooms you can see. A lazy
  // chunk drawn in the shell's dialog (which opens at once with a plain field and keeps what was typed). The box takes
  // `from:`, `in:`, `after:` and `before:`; the chips, the scope buttons and the date fields under it edit the same
  // text, so the two always agree. While the caret is in a `from:` or `in:` word, the field is a combobox over people
  // or rooms. Results are buttons in a region, not listbox options: Down moves real focus into them (a screen reader
  // reads the whole hit), Enter or a click jumps to the message, Up from the first or Esc goes back. Nothing animates.
  let { open, current, onclose, q = $bindable('') }: { open: boolean; current: number | null; onclose: () => void; q?: string } = $props();

  const s = new Searcher(search);
  const rooms = $derived([...session.state.rooms].sort((a, b) => a.position - b.position));
  const users = $derived(Object.values(session.state.users));
  const parsed = $derived(parseSearch(q, { users, rooms, today: new Date() }));
  const words = $derived(parsed.q.trim());
  const oneShort = $derived(!words.includes(' ') && shortLast(words)); // one word of one letter: not worth a request
  const ask = $derived(
    !open || parsed.unknown.length || nothingToSearch(words) || oneShort
      ? null
      : { q: words, room_id: parsed.room_id, author_id: parsed.author_id, from: parsed.from, to: parsed.to },
  );
  $effect(() => s.set(ask));
  $effect(() => () => s.stop()); // closing or signing out aborts a request in flight
  // A room you can no longer see (made private, or you left it): its hits go before the next paint, and the query runs
  // again. A new room runs it again too.
  let roomKey = '';
  $effect.pre(() => {
    const ids = new Set(rooms.map((r) => r.id));
    const key = [...ids].sort((a, b) => a - b).join();
    if (roomKey && key !== roomKey) untrack(() => s.keep(ids));
    roomKey = key;
  });
  const BUSY = 'Search is busy. Wait a moment, then press Enter to try again.';

  let field = $state<HTMLInputElement>();
  let caret = $state(0);
  let jumpError = $state('');
  $effect(() => {
    if (!open) [q, jumpError] = ['', ''];
    else {
      loadUsers().catch(() => {}); // the hello's list is still there
      if (field && document.activeElement !== field) {
        field.focus();
        field.setSelectionRange(q.length, q.length);
      }
    }
  });

  // Suggestions for the `from:` or `in:` word at the caret.
  const at = $derived(open ? keyAt(q, caret) : null);
  const sugs = $derived.by(() => {
    if (!at) return [];
    const t = at.q.toLowerCase();
    const has = (x: string) => x.toLowerCase().includes(t);
    return at.key === 'from'
      ? users.filter((u) => !u.banned && (has(u.username) || has(u.display_name))).slice(0, 6).map((u) => ({ id: u.user_id, value: u.username, label: u.display_name, user: u }))
      : rooms.filter((r) => has(r.name)).slice(0, 6).map((r) => ({ id: String(r.id), value: r.name, label: r.name, user: undefined }));
  });
  let active = $state(0);
  const uid = $props.id();
  const syncCaret = () => (caret = field?.selectionStart ?? q.length);
  function pick(i: number) {
    if (!at || !sugs[i]) return;
    const r = complete(q, at, sugs[i].value);
    q = r.text;
    void tick().then(() => {
      field?.setSelectionRange(r.caret, r.caret);
      syncCaret();
    });
  }

  let list = $state<HTMLElement>();
  const hitButtons = () => [...(list?.querySelectorAll<HTMLButtonElement>('button') ?? [])];
  function onkeydown(e: KeyboardEvent) {
    if (e.isComposing || e.keyCode === 229) return; // 229: Safari's Enter that ends an IME composition
    if (sugs.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        active = (active + (e.key === 'ArrowDown' ? 1 : -1) + sugs.length) % sugs.length;
      } else if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        pick(active);
      }
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      hitButtons()[0]?.focus();
    } else if (e.key === 'Enter' && s.status === 'busy') {
      e.preventDefault();
      s.retry();
    } else if (e.key === 'Enter' && s.status === 'done' && s.hits[0]) { // not a hit of the query before this one
      e.preventDefault();
      void go(s.hits[0]);
    }
  }
  // In the results: Up and Down step through the buttons, Up from the first goes back to the box, a letter types there.
  function onListKey(e: KeyboardEvent) {
    const b = hitButtons();
    const i = b.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const n = i + (e.key === 'ArrowDown' ? 1 : -1);
      (n < 0 ? field : b[Math.min(n, b.length - 1)])?.focus();
    } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== ' ') field?.focus();
  }

  const roomName = (id: number) => rooms.find((r) => r.id === id)?.name;
  const someone = { user_id: '', display_name: 'Someone', accent: '' };
  const author = (id: string) => session.state.users[id] ?? someone;
  const clock = new Intl.DateTimeFormat([], { hour: 'numeric', minute: '2-digit' });
  const now = $derived(open ? Date.now() : 0); // "Today" as of this open
  // Hits are newest first: a run of the same local day is one group.
  const groups = $derived(
    s.hits.reduce<{ day: string; hits: SearchHit[] }[]>((g, h) => {
      const day = dayLabel(h.message.created_at, now);
      if (g.at(-1)?.day !== day) g.push({ day, hits: [] });
      g.at(-1)!.hits.push(h);
      return g;
    }, []),
  );
  const count = $derived(`${s.hits.length}${s.next !== null ? '+' : ''} ${s.hits.length === 1 && s.next === null ? 'message' : 'messages'}`);

  async function go(h: SearchHit) {
    const { room_id, id } = h.message;
    jumpError = '';
    chat.jump = id; // set before the load: the timeline scrolls to it and flashes it as soon as the page lands
    try {
      await jumpTo(room_id, id);
    } catch (e) {
      chat.jump = null;
      jumpError = errorText(e);
      return;
    }
    onclose();
    navigate(href({ name: 'room', id: room_id })); // the same room: nothing changes but the slice already loaded
  }

  const CHIP: Record<Key, string> = { from: 'From', in: 'In', after: 'On or after', before: 'On or before' };
  const curName = $derived(rooms.find((r) => r.id === current)?.name);
  const scope = (here: boolean) => () => (q = setKey(q, 'in', here ? curName! : null));
  const setDate = (key: 'after' | 'before') => (e: Event) => (q = setKey(q, key, (e.currentTarget as HTMLInputElement).value || null));
</script>

<SwitcherField bind:value={q} bind:el={field} aria-label="Search messages" placeholder="Search messages. Try from:alice in:general"
  oninput={() => { syncCaret(); active = 0; }} onkeyup={syncCaret} onclick={syncCaret} {onkeydown}
  maxlength={MAX_QUERY} role="combobox" aria-expanded={sugs.length > 0} aria-controls={sugs.length ? `${uid}-sugs` : undefined} aria-autocomplete="list"
  aria-activedescendant={sugs.length ? `${uid}-s${active}` : undefined} aria-describedby="{uid}-help" />

<div class="flex shrink-0 flex-wrap items-center gap-2 border-b border-subtle px-4 py-2 text-xs text-fg-2">
  {#if curName}
    <div class="seg" role="group" aria-label="Where to search">
      <button type="button" aria-pressed={parsed.room_id === null} onclick={scope(false)}>All rooms</button>
      <button type="button" aria-pressed={parsed.room_id === current} onclick={scope(true)}>This room</button>
    </div>
  {/if}
  <!-- Dates show in their own fields, not as chips. -->
  {#each parsed.chips.filter((c) => c.key === 'from' || c.key === 'in') as c (c.key)}
    <span class="chip">{CHIP[c.key]} <span class="font-medium text-fg">{c.text}</span>
      <button type="button" class="x" aria-label="Remove {CHIP[c.key].toLowerCase()} {c.text}" onclick={() => { q = setKey(q, c.key, null); field?.focus(); }}><Icon d={I.x} size={16} /></button>
    </span>
  {/each}
  <span class="ml-auto flex items-center gap-2">
    {#each [['after', 'After'], ['before', 'Before']] as const as [key, label] (key)}
      <label class="flex items-center gap-1">{label} <input type="date" class="control h-7 px-1.5 text-xs" value={keyValue(q, key)} onchange={setDate(key)} /></label>
      {#if keyValue(q, key)}
        <button type="button" class="x -ml-1" aria-label="Clear the {label.toLowerCase()} date" onclick={() => { q = setKey(q, key, null); field?.focus(); }}><Icon d={I.x} size={16} /></button>
      {/if}
    {/each}
  </span>
</div>

{#if sugs.length}
  <div id="{uid}-sugs" role="listbox" aria-label={at?.key === 'from' ? 'People' : 'Rooms'} class="min-h-0 flex-1 overflow-y-auto p-2">
    {#each sugs as o, i (o.id)}
      <!-- Clicks pick a row; keys are handled by the field, which keeps focus (the combobox pattern). -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div id="{uid}-s{i}" role="option" aria-selected={i === active} tabindex="-1" class="opt" class:on={i === active}
        onpointermove={() => (active = i)} onpointerdown={(e) => e.preventDefault()} onclick={() => pick(i)}>
        {#if o.user}<Avatar user={o.user} size={20} />{:else}<span class="flex w-5 justify-center text-fg-3"><Icon d={I.room} /></span>{/if}
        <span class="min-w-0 truncate text-fg">{o.label}</span>
        {#if o.user}<span class="truncate text-xs text-fg-3">@{o.value}</span>{/if}
      </div>
    {/each}
  </div>
{:else}
  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if parsed.unknown.length}
      <div class="px-4 py-6 text-center text-fg-2">
        {#each parsed.unknown as u (u)}<p>{u}</p>{/each}
        {#if parsed.unknown.some((u) => u.startsWith('No '))}<p class="mt-1 text-xs text-fg-3">Type from: or in: and pick a name from the list.</p>{/if}
      </div>
    {:else if !q.trim()}
      <div class="px-4 py-6 text-fg-2">
        <p class="mb-3 text-fg">Find messages in the rooms you can see, newest first.</p>
        <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-2 text-fg-3">
          <dt><code>from:alice</code></dt><dd>by one person (quote a name with spaces)</dd>
          <dt><code>in:general</code></dt><dd>in one room</dd>
          <dt><code>after:2026-10-01</code></dt><dd>on or after that day</dd>
          <dt><code>before:2026-10-31</code></dt><dd>on or before that day</dd>
        </dl>
      </div>
    {:else if !words}
      <p class="px-4 py-6 text-center text-fg-2">Add a word to search for.</p>
    {:else if oneShort}
      <p class="px-4 py-6 text-center text-fg-2">Type at least 2 letters to search as you type.</p>
    {:else if nothingToSearch(words) || (s.status === 'done' && !s.hits.length)}
      <div class="px-4 py-6 text-center text-fg-2">
        <p>No messages found.</p>
        <p class="mt-1 text-xs text-fg-3">Try other words, or fewer filters.</p>
      </div>
    {:else if s.status === 'error'}
      <p class="px-4 py-6 text-center text-danger" role="alert">{s.error}</p>
    {:else if s.status === 'busy' && !s.hits.length}
      <div class="flex flex-col items-center gap-3 px-4 py-6 text-center text-fg-2">
        <p>{BUSY}</p>
        <Button size="sm" onclick={() => s.retry()}>Try again</Button>
      </div>
    {:else if !s.hits.length}
      <p class="px-4 py-6 text-center text-fg-2">Searching...</p>
    {:else}
      <div class="flex h-8 items-center gap-3 px-4 text-xs text-fg-3">
        <span>{s.status === 'done' ? count : s.status === 'busy' ? BUSY : 'Searching...'}</span>
        {#if s.status === 'busy'}<button type="button" class="font-medium text-fg-2 hover:text-fg" onclick={() => s.retry()}>Try again</button>{/if}
        {#if shortLast(words)}<span>Type at least 2 letters to search as you type.</span>{/if}
      </div>
      {#if jumpError}<p class="px-4 pb-2 text-xs text-danger" role="alert">{jumpError}</p>{/if}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div bind:this={list} role="region" aria-label="Search results" class="px-2 pb-2" onkeydown={onListKey}>
        {#each groups as g (g.day)}
          <h3 class="flex h-7 items-center px-2 text-xs font-medium text-fg-3">{g.day}</h3>
          {#each g.hits as h (h.message.id)}
            {@const a = author(h.message.author_id)}
            <button type="button" class="hit" onclick={() => go(h)}>
              <Avatar user={a} size={32} />
              <span class="min-w-0 flex-1">
                <span class="flex items-baseline gap-2">
                  <span class="truncate font-medium text-fg">{a.display_name}</span>
                  {#if roomName(h.message.room_id)}<span class="truncate text-xs text-fg-3">in {roomName(h.message.room_id)}</span>{/if}
                  <time class="shrink-0 text-xs text-fg-3 tabular-nums">{clock.format(h.message.created_at)}</time>
                  <span class="go">Jump <Icon d={I.chevronRight} size={16} /></span>
                </span>
                <span class="snip">{#each marks(h.snippet) as part}{#if part.mark}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}</span>
              </span>
            </button>
          {/each}
        {/each}
        {#if s.next !== null}
          <div class="flex justify-center pt-2">
            <Button size="sm" loading={s.paging} onclick={() => s.more()}>More results</Button>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<div class="flex h-9 shrink-0 items-center gap-4 border-t border-subtle px-4 text-xs text-fg-3">
  <span class="flex items-center gap-1" aria-hidden="true"><Kbd>↑</Kbd><Kbd>↓</Kbd> move</span>
  <span class="flex items-center gap-1" aria-hidden="true"><Kbd>Enter</Kbd> {sugs.length ? 'pick' : s.status === 'busy' ? 'try again' : 'jump'}</span>
  <span class="flex items-center gap-1" aria-hidden="true"><Kbd>Esc</Kbd> close</span>
  <span id="{uid}-help" class="ml-auto">Dates are whole days in your time zone.</span>
</div>
<p role="status" class="sr-only">{!open || !ask ? '' : s.status === 'busy' ? BUSY : s.status === 'done' ? (s.hits.length ? count : 'No messages found') : ''}</p>

<style>
  code { color: var(--fg-2); font-family: var(--font-mono); font-size: 12px; }
  .seg { display: flex; padding: 2px; border-radius: var(--radius); background: var(--bg-element); }
  .seg button { height: 24px; padding: 0 8px; border-radius: var(--radius-sm); color: var(--fg-2); }
  .seg button:hover { color: var(--fg); }
  .seg [aria-pressed='true'] { background: var(--bg-selected); color: var(--fg); }
  .chip { display: inline-flex; height: 28px; align-items: center; gap: 4px; padding: 0 2px 0 8px; border-radius: var(--radius-sm); background: var(--accent-subtle); color: var(--accent-fg); }
  .x { display: flex; padding: 4px; border-radius: var(--radius-sm); }
  .x:hover { background: var(--bg-hover); color: var(--fg); }
  .chip .x:hover { background: var(--accent-line); }
  .opt { display: flex; height: 36px; align-items: center; gap: 8px; padding: 0 8px; border-radius: var(--radius); }
  .opt.on { background: var(--line); }
  .hit { display: flex; width: 100%; gap: 12px; padding: 8px; border-radius: var(--radius); text-align: left; }
  .hit:hover, .hit:focus-visible { background: var(--bg-hover); }
  .hit:focus-visible { outline-offset: -2px; }
  .go { display: flex; align-items: center; gap: 2px; margin-left: auto; color: var(--fg-2); font-size: 12px; opacity: 0; }
  .hit:hover .go, .hit:focus-visible .go { opacity: 1; }
  .snip { display: block; margin-top: 2px; color: var(--fg-2); overflow-wrap: anywhere; }
  mark { border-radius: 2px; background: var(--accent-subtle); color: var(--accent-fg); }
</style>
