<script lang="ts">
  import * as I from '../lib/ui/icons';
  import { tick } from 'svelte';
  import { ADMIN_NAMES, ADMIN_OWNER_ONLY, ADMIN_SECTIONS, SETTINGS_NAMES, SETTINGS_SECTIONS, href } from '../lib/route';
  import { navigate, session } from '../lib/session.svelte';
  import { join, leave, toggleDeafen, toggleMute, voice } from '../lib/voice.svelte';
  import { layers } from '../lib/ui/layers.svelte';
  import { openPerson } from '../lib/ui/person.svelte';
  import { loadUsers } from '../lib/ui/users';
  import { setCollapsed, sidebar } from '../lib/ui/sidebar.svelte';
  import { isMac, keysOf, nextUnread, type Shortcut } from '../lib/ui/keys';
  import { rank, type Item } from '../lib/ui/switcher';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Badge from '../lib/ui/c/Badge.svelte';
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import Kbd from '../lib/ui/c/Kbd.svelte';
  import Shortcuts from './Shortcuts.svelte';
  // Ctrl/Cmd+K: one text field over a list of rooms, people and actions (a combobox and a listbox; focus stays in the
  // field and aria-activedescendant points at the active row). It opens and moves without motion: it is used many times
  // a day. A lazy chunk, loaded on the first open; the shell keeps it mounted after that so Esc can give focus back. The
  // shortcuts list, opened from here, rides in the same chunk.
  let { open, current, recent, onclose }: { open: boolean; current: number | null; recent: number[]; onclose: () => void } = $props();

  const mac = isMac();
  const rooms = $derived([...session.state.rooms].sort((a, b) => a.position - b.position));
  const me = $derived(session.state.me?.user);
  const roomName = (id: number) => rooms.find((r) => r.id === id)?.name ?? 'a room';
  const where = (uid: string) => {
    const r = uid === me?.user_id ? voice.joinedRoom : session.state.voice.find((p) => p.user_id === uid)?.room_id;
    return r == null ? undefined : `In voice in ${roomName(r)}`;
  };

  const go = (path: string) => () => navigate(path);
  const act = (id: string, label: string, run: () => void, keys?: Shortcut): Item => ({ kind: 'action', id, label, run, hint: keys });
  const items = $derived.by((): Item[] => {
    const room = rooms.find((r) => r.id === current);
    const people = Object.values(session.state.users).filter((u) => !u.banned && u.user_id !== me?.user_id);
    const inVoice = (u: { user_id: string }) => (where(u.user_id) ? 0 : 1);
    people.sort((a, b) => inVoice(a) - inVoice(b) || a.display_name.localeCompare(b.display_name));
    const next = nextUnread(rooms.map((r) => r.id), current, session.state.unread);
    const staff = me?.role === 'owner' || me?.role === 'moderator';
    return [
      ...rooms.map((r): Item => ({ kind: 'room', id: String(r.id), label: r.name, run: go(href({ name: 'room', id: r.id })) })),
      ...people.map((u): Item => ({ kind: 'person', id: u.user_id, label: u.display_name, hint: where(u.user_id), run: () => openPerson(u.user_id) })),
      ...(room && voice.joinedRoom !== room.id
        ? [act('join', `Join voice in ${room.name}`, () => { if (voice.joined) leave(); void join(room.id); })]
        : []),
      ...(voice.joined
        ? [
            act('mute', voice.muted ? 'Unmute' : 'Mute', toggleMute, 'mute'),
            act('deafen', voice.deafened ? 'Undeafen' : 'Deafen', toggleDeafen, 'deafen'),
            act('leave', 'Leave voice', leave),
          ]
        : []),
      ...(room ? [act('search', 'Search messages', focusSearch, 'search')] : []),
      ...(next !== null ? [act('next-unread', `Next unread room: ${roomName(next)}`, go(href({ name: 'room', id: next })), 'next-unread')] : []),
      ...SETTINGS_SECTIONS.map((s) => act(`settings-${s}`, `Settings: ${SETTINGS_NAMES[s]}`, go(href({ name: 'settings', section: s })))),
      ...(staff ? ADMIN_SECTIONS : [])
        .filter((s) => me?.role === 'owner' || !ADMIN_OWNER_ONLY.includes(s))
        .map((s) => act(`admin-${s}`, `Admin: ${ADMIN_NAMES[s]}`, go(href({ name: 'admin', section: s })))),
      act('sidebar', sidebar.collapsed ? 'Show sidebar' : 'Hide sidebar', () => setCollapsed(!sidebar.collapsed), 'sidebar'),
      act('shortcuts', 'Keyboard shortcuts', () => (layers.shortcuts = true)),
      act('logout', 'Log out', () => void session.logout()),
    ];
  });

  let q = $state('');
  let active = $state(0);
  const results = $derived(open ? rank(q, items, { unread: session.state.unread, recent: recent.filter((id) => id !== current) }) : []);
  const uid = $props.id();
  const optId = (i: number) => `${uid}-o${i}`;

  // Each open starts empty, at the top. People who signed up after this tab connected come from /api/users (once).
  $effect(() => {
    if (open) {
      [q, active] = ['', 0];
      loadUsers().catch(() => {}); // the list from hello is still there
    }
  });
  // The count is read once the typing pauses, not on every key.
  let said = $state('');
  $effect(() => {
    const n = results.length;
    const t = setTimeout(() => (said = n ? `${n} result${n === 1 ? '' : 's'}` : 'No results'), 600);
    return () => clearTimeout(t);
  });

  function moveTo(i: number) {
    active = Math.max(0, Math.min(results.length - 1, i));
    void tick().then(() => document.getElementById(optId(active))?.scrollIntoView({ block: 'nearest' }));
  }
  function onkeydown(e: KeyboardEvent) {
    if (e.isComposing) return;
    const n = results.length;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (n) moveTo((active + (e.key === 'ArrowDown' ? 1 : -1) + n) % n);
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault(); // short text, so the row matters more than the caret
      moveTo(e.key === 'Home' ? 0 : n - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (results[active]) choose(results[active]);
    }
  }
  function choose(it: Item) {
    onclose();
    // After the dialog gave focus back, so a new dialog (a person card) or the search field takes it from there.
    void tick().then(() => it.run());
  }
  function focusSearch() {
    document.querySelector<HTMLInputElement>('input[aria-label="Search messages"]')?.focus();
  }
  const GROUP = { room: 'Rooms', person: 'People', action: 'Actions' } as const;
  // Action rows: an icon by the first part of their id.
  const ICONS: Record<string, I.IconPath> = {
    join: I.headphones, mute: I.micOff, deafen: I.headphonesOff, leave: I.phoneOff, search: I.search, next: I.arrowDown,
    settings: I.settings, admin: I.shield, sidebar: I.sidebar, shortcuts: I.command, logout: I.logOut,
  };
  const unreadText = (id: string) => {
    const u = session.state.unread[Number(id)];
    return [u?.unread && `${u.unread} unread`, u?.mentions && `${u.mentions} mention${u.mentions === 1 ? '' : 's'}`].filter(Boolean).join(', ');
  };
</script>

<Dialog {open} title="Switch to" bare {onclose}>
  <div class="field flex h-12 shrink-0 items-center gap-3 border-b border-subtle px-4 text-fg-3">
    <Icon d={I.search} size={20} />
    <input class="h-full min-w-0 flex-1 bg-transparent text-lg text-fg placeholder:text-fg-3" bind:value={q}
      oninput={() => (active = 0)} {onkeydown} placeholder="Rooms, people and actions" aria-label="Switch to" autocomplete="off" spellcheck="false"
      role="combobox" aria-expanded={results.length > 0} aria-controls="{uid}-list" aria-autocomplete="list"
      aria-activedescendant={results.length ? optId(active) : undefined} />
  </div>

  <div id="{uid}-list" role="listbox" aria-label="Results" class="min-h-0 flex-1 overflow-y-auto p-2" class:hidden={!results.length}>
    {#each results as it, i (it.kind + it.id)}
      {#if i === 0 || results[i - 1].kind !== it.kind}
        <div role="presentation" class="flex h-7 items-center px-2 text-xs font-medium text-fg-3" class:mt-2={i > 0}>{GROUP[it.kind]}</div>
      {/if}
      {@const u = it.kind === 'room' ? session.state.unread[Number(it.id)] : undefined}
      {@const user = it.kind === 'person' ? session.state.users[it.id] : undefined}
      <!-- Clicks run a row; keys are handled by the field, which keeps focus (the combobox pattern). -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div id={optId(i)} role="option" aria-selected={i === active} class="opt" class:on={i === active} tabindex="-1"
        onpointermove={() => (active = i)} onclick={() => choose(it)}>
        {#if it.kind === 'room'}
          <span class="lead"></span>
        {:else if user}
          <Avatar {user} size={20} />
        {:else}
          <span class="lead"><Icon d={ICONS[it.id.split('-')[0]] ?? I.chevronRight} /></span>
        {/if}
        <span class="min-w-0 truncate text-fg">{it.label}</span>
        {#if it.kind === 'room' && unreadText(it.id)}<span class="sr-only">, {unreadText(it.id)}</span>{/if}
        {#if it.kind === 'person' && it.hint}<span class="sr-only">, {it.hint}</span>{/if}
        <span class="ml-auto flex shrink-0 items-center gap-1 pl-3">
          {#if u?.unread}<Badge kind="count" n={u.unread} />{/if}
          {#if u?.mentions}<Badge kind="mention" n={u.mentions} />{/if}
          {#if it.kind === 'person' && it.hint}<span class="meta text-xs" aria-hidden="true">{it.hint}</span>{/if}
          {#if it.kind === 'action' && it.hint}
            <span class="flex gap-1" aria-hidden="true">{#each keysOf(it.hint as Shortcut, mac) as k (k)}<Kbd>{k}</Kbd>{/each}</span>
          {/if}
        </span>
      </div>
    {/each}
  </div>
  {#if !results.length}
    <p class="px-4 py-6 text-center text-fg-2">Nothing matches “{q.trim()}”.</p>
  {/if}

  <div class="flex h-9 shrink-0 items-center gap-4 border-t border-subtle px-4 text-xs text-fg-3" aria-hidden="true">
    <span class="flex items-center gap-1"><Kbd>↑</Kbd><Kbd>↓</Kbd> move</span>
    <span class="flex items-center gap-1"><Kbd>Enter</Kbd> open</span>
    <span class="flex items-center gap-1"><Kbd>Esc</Kbd> close</span>
    <span class="ml-auto flex items-center gap-1"><Kbd>r</Kbd> rooms <Kbd>p</Kbd> people <Kbd>&gt;</Kbd> actions</span>
  </div>
  <p role="status" class="sr-only">{open ? said : ''}</p>
</Dialog>

<style>
  .opt {
    display: flex;
    height: 36px;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-radius: var(--radius);
    color: var(--fg-2);
    cursor: default;
  }
  .opt.on { background: var(--bg-selected); }
  /* The count pill is drawn on --bg-selected: lift it on the active row so it does not vanish. */
  .opt.on :global(.bg-selected) { background: var(--line); }
  /* The field always has focus while the dialog is open: the caret and an amber line under it show it, not a ring. */
  .field input:focus-visible { outline: none; }
  .field:focus-within { border-bottom-color: var(--accent-line); }
  .lead { display: flex; width: 20px; justify-content: center; color: var(--fg-3); }
  .meta { color: var(--fg-3); }
  /* --fg-3 is under 4.5:1 on the selected surface: step up to --fg-2 there */
  .on .lead, .on .meta { color: var(--fg-2); }
</style>

<Shortcuts open={layers.shortcuts} onclose={() => (layers.shortcuts = false)} />
