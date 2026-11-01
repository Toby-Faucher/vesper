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
  import { SCOPES, rank, type Item } from '../lib/ui/switcher';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Badge from '../lib/ui/c/Badge.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import Kbd from '../lib/ui/c/Kbd.svelte';
  import SwitcherField from './SwitcherField.svelte';
  // Ctrl/Cmd+K: one text field over a list of rooms, people and actions (a combobox and a listbox; focus stays in the
  // field and aria-activedescendant points at the active row). It opens and moves without motion: it is used many times
  // a day. A lazy chunk, loaded on the first open, drawn in the shell's dialog, which opens at once with a plain field;
  // `q` is that field's text, so what was typed while the chunk loaded is kept. The shell keeps it mounted after that.
  let { open, current, recent, onclose, q = $bindable('') }:
    { open: boolean; current: number | null; recent: number[]; onclose: () => void; q?: string } = $props();

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

  let active = $state(0);
  // "r ", "p " or "> " typed first becomes a chip (Backspace at the start removes it); `q` keeps the rest.
  let scope = $state('');
  $effect(() => {
    const p = q.slice(0, 2).toLowerCase();
    if (!scope && SCOPES[p]) [scope, q] = [p, q.slice(2)];
  });
  const SCOPE_NAMES: Record<string, string> = { 'r ': 'Rooms', 'p ': 'People', '> ': 'Actions' };
  const results = $derived(open ? rank(scope + q, items, { unread: session.state.unread, recent: recent.filter((id) => id !== current) }) : []);
  // Runs of one kind, for the listbox groups. `i` is the row's place in `results` (the active row and its id).
  const groups = $derived(
    results.reduce<{ kind: Item['kind']; rows: { it: Item; i: number }[] }[]>((g, it, i) => {
      if (g.at(-1)?.kind !== it.kind) g.push({ kind: it.kind, rows: [] });
      g.at(-1)!.rows.push({ it, i });
      return g;
    }, []),
  );
  const uid = $props.id();
  const optId = (i: number) => `${uid}-o${i}`;
  let field = $state<HTMLInputElement>();

  // Each open starts empty, at the top, with nothing to announce. People who signed up after this tab connected come
  // from /api/users (once). On the first open the field replaces the shell's loading field: focus it, caret at the end.
  $effect(() => {
    if (!open) [q, scope, active, said] = ['', '', 0, ''];
    else {
      loadUsers().catch(() => {}); // the list from hello is still there
      if (field && document.activeElement !== field) {
        field.focus();
        field.setSelectionRange(q.length, q.length);
      }
    }
  });
  // The count is read once the typing pauses, not on every key.
  let said = $state('');
  $effect(() => {
    if (!open) return;
    const n = results.length;
    const t = setTimeout(() => (said = n ? `${n} result${n === 1 ? '' : 's'}` : 'No results'), 600);
    return () => clearTimeout(t);
  });

  function moveTo(i: number) {
    active = Math.max(0, Math.min(results.length - 1, i));
    void tick().then(() => document.getElementById(optId(active))?.scrollIntoView({ block: 'nearest' }));
  }
  function onkeydown(e: KeyboardEvent) {
    if (e.isComposing || e.keyCode === 229) return; // 229: Safari's Enter that ends an IME composition
    const n = results.length;
    if (e.key === 'Backspace' && scope && field?.selectionStart === 0 && field.selectionEnd === 0) {
      e.preventDefault();
      [scope, active] = ['', 0];
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
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

<SwitcherField bind:value={q} bind:el={field} oninput={() => (active = 0)} {onkeydown}
  role="combobox" aria-expanded={results.length > 0} aria-controls="{uid}-list" aria-autocomplete="list"
  aria-activedescendant={results.length ? optId(active) : undefined} aria-describedby={scope ? `${uid}-scope` : undefined}
  placeholder={scope ? `Find ${SCOPE_NAMES[scope].toLowerCase()}` : 'Rooms, people and actions'}>
  {#snippet lead()}
    {#if scope}<span id="{uid}-scope" class="chip">{SCOPE_NAMES[scope]}<span class="sr-only"> only</span></span>{/if}
  {/snippet}
</SwitcherField>

<div id="{uid}-list" role="listbox" aria-label="Results" class="min-h-0 flex-1 overflow-y-auto p-2" class:hidden={!results.length}>
  {#each groups as g, n (g.kind)}
    <div role="group" aria-labelledby="{uid}-g-{g.kind}">
      <div id="{uid}-g-{g.kind}" role="presentation" class="flex h-7 items-center px-2 text-xs font-medium text-fg-3" class:mt-2={n > 0}>{GROUP[g.kind]}</div>
      {#each g.rows as { it, i } (it.kind + it.id)}
        {@const u = it.kind === 'room' ? session.state.unread[Number(it.id)] : undefined}
        {@const user = it.kind === 'person' ? session.state.users[it.id] : undefined}
        <!-- Clicks run a row; keys are handled by the field, which keeps focus (the combobox pattern). -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div id={optId(i)} role="option" aria-selected={i === active} class="opt" class:on={i === active} tabindex="-1"
          onpointermove={() => (active = i)} onclick={() => choose(it)}>
          {#if user}
            <Avatar {user} size={20} />
          {:else}
            <span class="lead"><Icon d={it.kind === 'room' ? I.room : (ICONS[it.id.split('-')[0]] ?? I.chevronRight)} /></span>
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
  {/each}
</div>
{#if !results.length}
  <p class="px-4 py-6 text-center text-fg-2">{q.trim() ? `Nothing matches “${q.trim()}”.` : 'Nothing to show here.'}</p>
{/if}

<div class="flex h-9 shrink-0 items-center gap-4 border-t border-subtle px-4 text-xs text-fg-3" aria-hidden="true">
  <span class="flex items-center gap-1"><Kbd>↑</Kbd><Kbd>↓</Kbd> move</span>
  <span class="flex items-center gap-1"><Kbd>Enter</Kbd> open</span>
  <span class="flex items-center gap-1"><Kbd>Esc</Kbd> close</span>
  <span class="ml-auto flex items-center gap-1">Filter: <Kbd>r</Kbd> rooms <Kbd>p</Kbd> people <Kbd>&gt;</Kbd> actions, then <Kbd>Space</Kbd></span>
</div>
<p role="status" class="sr-only">{open ? said : ''}</p>

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
  /* The keyboard's row: one step past --bg-selected (1.5:1 on the dialog, text 9.9:1), so it reads without the text colour. */
  .opt.on { background: var(--line); }
  /* The count pill is drawn on --bg-selected: lift it on the active row so it does not vanish. */
  .opt.on :global(.bg-selected) { background: var(--line-strong); color: var(--fg); }
  .chip {
    display: inline-flex;
    height: 24px;
    flex-shrink: 0;
    align-items: center;
    padding: 0 8px;
    border-radius: var(--radius-sm);
    background: var(--accent-subtle);
    color: var(--accent-fg);
    font-size: 14px;
    font-weight: 500;
  }
  .lead { display: flex; width: 20px; justify-content: center; color: var(--fg-3); }
  .meta { color: var(--fg-3); }
  /* --fg-3 is under 4.5:1 on the selected surface: step up to --fg-2 there */
  .on .lead, .on .meta { color: var(--fg-2); }
</style>

