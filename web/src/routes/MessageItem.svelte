<script lang="ts" module>
  // One clock for the head and the gutter: "7:41 AM", no leading zero. Made once, not per row.
  const clock = new Intl.DateTimeFormat([], { hour: 'numeric', minute: '2-digit' });
</script>

<script lang="ts">
  // One message row. A head row has the avatar, the name and the time; a continuation row shows its time in the gutter
  // on hover. The toolbar overlays the top right and shows on hover and on keyboard focus inside the row (opacity only,
  // so nothing moves and it stays clickable). It is one Tab stop (roving: Left and Right move inside it), and Up and Down
  // move between rows. Its menu, the reaction picker and the delete dialog mount on first use.
  // Names and bodies are untrusted: text nodes and Markdown.svelte only. User colour only on the avatar.
  import { tick } from 'svelte';
  import Attachments from './Attachments.svelte';
  import Markdown from './Markdown.svelte';
  import { parse } from '../lib/markdown';
  import { chat, edit, pin, react, remove } from '../lib/chat.svelte';
  import type { Message } from '../lib/gen/Message';
  import * as I from '../lib/ui/icons';
  import { isLong, plain } from '../lib/ui/timeline';
  import { isSendKey } from '../lib/ui/keys';
  import { copyText } from '../lib/ui/clipboard';
  import { toolMove } from '../lib/ui/menu';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import Menu from '../lib/ui/c/Menu.svelte';
  import MenuItem from '../lib/ui/c/MenuItem.svelte';
  import Popover from '../lib/ui/c/Popover.svelte';
  import { motion } from '../lib/ui/motion';

  let {
    m,
    head,
    myId,
    myName,
    isMod,
    user,
    act,
    onreply,
    onjump,
    onsay,
    onview,
  }: {
    m: Message;
    head: boolean;
    myId: string;
    myName: string;
    isMod: boolean;
    user: (id: string) => { user_id: string; display_name: string; accent: string };
    act: (f: () => Promise<unknown>, conflict?: string) => Promise<boolean>;
    onreply: (m: Message) => void;
    onjump: (id: number) => void;
    onsay: (text: string) => void;
    onview: (id: string) => void;
  } = $props();

  const EMOJIS = ['👍', '😂', '❤️'];
  const author = $derived(user(m.author_id));
  const mine = $derived(m.author_id === myId);
  const mentioned = $derived(!m.deleted && m.mentions.includes(myId));
  const nodes = $derived(parse(m.body));
  const long = $derived(isLong(m.body));
  const hm = (ms: number) => clock.format(ms);
  const full = (ms: number) => new Date(ms).toLocaleString([], { dateStyle: 'full', timeStyle: 'short' });
  // The gutter has room for "7:26", not "07:26 AM": the hour and minute only (the title has the full date).
  const short = (ms: number) =>
    clock.formatToParts(ms).filter((p) => p.type !== 'dayPeriod').map((p) => p.value).join('').trim();
  const iso = (ms: number) => new Date(ms).toISOString();
  const names = (ids: string[]) => ids.map((id) => user(id).display_name).join(', ');

  let expanded = $state(false);
  let editing = $state(false);
  let editText = $state('');
  let saving = $state(false);
  let moreEl = $state<HTMLButtonElement | null>(null);
  let reactEl = $state<HTMLButtonElement | null>(null);
  let menu = $state({ mounted: false, open: false });
  let picker = $state({ mounted: false, open: false });
  let confirm = $state({ mounted: false, open: false });
  let symbol = $state('');

  const toggle = (e: string, on: boolean) => act(() => react(m.id, e, on));
  const has = (e: string) => m.reactions.find((r) => r.emoji === e)?.user_ids.includes(myId) ?? false;

  let fromComposer = false; // opened by Up in the composer: focus goes back there
  function startEditing(composer = false) {
    editText = m.body;
    editing = true;
    fromComposer = composer;
  }
  $effect(() => {
    if (chat.edit !== m.id) return;
    chat.edit = null;
    if (!m.deleted) startEditing(true);
  });

  async function stopEditing() {
    editing = false;
    await tick();
    if (fromComposer) chat.refocus++;
    else moreEl?.focus(); // Edit lives in the More menu: focus goes back to the button that opened it
  }
  async function save() {
    if (!editText.trim() || saving) return;
    saving = true;
    const ok = await act(() => edit(m.id, editText));
    saving = false;
    if (ok) await stopEditing();
  }
  function onEditKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation(); // the Esc that cancels an edit does not also jump the timeline
      void stopEditing();
    } else if (isSendKey(e)) {
      e.preventDefault();
      void save();
    }
  }
  function pick(e: string) {
    picker.open = false;
    void toggle(e, !has(e));
  }
  function pickTyped(e: SubmitEvent) {
    e.preventDefault();
    const s = symbol.trim();
    if (!s) return;
    symbol = '';
    pick([...new Intl.Segmenter().segment(s)][0].segment); // one symbol, as typed
  }
  const copy = (text: string) => copyText(text).then((ok) => ok || onsay(UI_MESSAGES.copyFailed));

  // Keys in the row (Ruling in the Task 7 fix report): Left, Right, Home and End move along the toolbar; Up and Down go
  // to the previous or next message row. Not while typing in the edit box.
  let tool = $state(0); // the toolbar button that is the row's Tab stop
  let toolsEl = $state<HTMLElement>();
  const tbs = () => [...(toolsEl?.querySelectorAll<HTMLButtonElement>('.tb') ?? [])];
  function onRowKey(e: KeyboardEvent) {
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey || e.isComposing) return;
    const t = e.target as HTMLElement;
    if (t.closest('textarea, input')) return;
    const row = e.currentTarget as HTMLElement;
    if (toolsEl?.contains(t)) {
      const i = toolMove(e.key, tool, tbs().length);
      if (i !== null) {
        e.preventDefault();
        tool = i;
        tbs()[i].focus();
        return;
      }
    }
    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
    let next = row[e.key === 'ArrowUp' ? 'previousElementSibling' : 'nextElementSibling'];
    while (next && next.tagName !== 'ARTICLE') next = next[e.key === 'ArrowUp' ? 'previousElementSibling' : 'nextElementSibling'];
    if (!(next instanceof HTMLElement)) return; // the first or last row: the list scrolls as usual
    e.preventDefault();
    next.focus({ preventScroll: true });
    next.scrollIntoView({ block: 'nearest' });
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions: a row takes focus from Up and Down -->
<article id="m{m.id}" class="msg" class:head class:mentioned class:editing tabindex="-1" onkeydown={onRowKey}>
  {#if m.reply_to}
    {@const r = m.reply_to}
    <button type="button" class="quote quiet" onclick={() => onjump(r.id)}>
      <Icon d={I.reply} />
      <span class="truncate">Reply to <span class="text-fg-2">{user(r.author_id).display_name}</span>: {plain(parse(r.excerpt)) || 'a deleted message'}</span>
    </button>
  {/if}
  <div class="gutter">
    {#if head}
      <Avatar user={author} size={32} />
    {:else}
      <time class="gtime" datetime={iso(m.created_at)} title={full(m.created_at)}>{short(m.created_at)}</time>
    {/if}
  </div>
  <div class="content min-w-0">
    {#if head}
      <p class="meta">
        <span class="font-medium text-fg">{author.display_name}</span>
        <time class="quiet text-xs tabular-nums" datetime={iso(m.created_at)} title={full(m.created_at)}>{hm(m.created_at)}</time>
        {#if m.pinned}<span class="quiet flex items-center gap-1 text-xs"><Icon d={I.pin} size={16} />Pinned</span>{/if}
      </p>
    {/if}
    {#if m.deleted}
      <p class="quiet italic">Message deleted</p>
    {:else if editing}
      <textarea class="control mt-1 block w-full resize-none px-2.5 py-1.5" rows="2" aria-label="Edit message" bind:value={editText}
        onkeydown={onEditKey} {@attach (el) => { el.focus({ preventScroll: true }); el.closest('article')?.scrollIntoView({ block: 'nearest' }); }}></textarea>
      <p class="mt-1 flex items-center gap-2 text-xs text-fg-2">
        <Button size="sm" variant="primary" loading={saving} onclick={save}>Save</Button>
        <Button size="sm" variant="ghost" onclick={stopEditing}>Cancel</Button>
        Enter saves, Esc cancels.
      </p>
    {:else}
      <div class="body" class:clamp={long && !expanded}>
        <Markdown {nodes} me={myName} />
        {#if m.edited_at || (!head && m.pinned)}
          <span class="note quiet">{m.edited_at ? '(edited)' : ''}{!head && m.pinned ? `${m.edited_at ? ' ' : ''}Pinned` : ''}</span>
        {/if}
      </div>
      {#if long}
        <button type="button" class="more-link" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>{expanded ? 'Show less' : 'Show more'}</button>
      {/if}
      {#if m.attachments.length}<Attachments list={m.attachments} {onview} />{/if}
      <!-- Always there (no height when empty), so the first reaction pops in too. -->
      <div class="flex flex-wrap gap-1" class:mt-1={m.reactions.length}>
          {#each m.reactions as r (r.emoji)}
            {@const on = r.user_ids.includes(myId)}
            <button type="button" class="pill" class:on aria-pressed={on} aria-label="React {r.emoji}, {r.user_ids.length}"
              title={names(r.user_ids)} onclick={() => toggle(r.emoji, !on)} in:motion={'bump'} out:motion={'bump'}>
              <span>{r.emoji}</span>{#key r.user_ids.length}<span class="inline-block tabular-nums" in:motion={'bump'}>{r.user_ids.length}</span>{/key}
            </button>
          {/each}
      </div>
    {/if}
  </div>

  {#if !m.deleted && !editing}
    <!-- Each button is described by whose message it acts on: "Reply", then "bob, 7:41 AM". -->
    <span id="m{m.id}-by" hidden>{author.display_name}, {hm(m.created_at)}</span>
    <div class="tools" role="toolbar" aria-label="Message actions" bind:this={toolsEl}
      onfocusin={(e) => { const i = tbs().indexOf(e.target as HTMLButtonElement); if (i >= 0) tool = i; }}>
      <button type="button" class="tb" aria-label="Add reaction" aria-describedby="m{m.id}-by" tabindex={tool === 0 ? 0 : -1} bind:this={reactEl} onclick={() => (picker = { mounted: true, open: !picker.open })}><Icon d={I.smile} /></button>
      <button type="button" class="tb" aria-label="Reply" aria-describedby="m{m.id}-by" tabindex={tool === 1 ? 0 : -1} onclick={() => onreply(m)}><Icon d={I.reply} /></button>
      <button type="button" class="tb" aria-label={m.pinned ? 'Unpin' : 'Pin'} aria-describedby="m{m.id}-by" tabindex={tool === 2 ? 0 : -1} onclick={() => act(() => pin(m.id, !m.pinned), UI_MESSAGES.pinsFull)}><Icon d={I.pin} /></button>
      <button type="button" class="tb" aria-label="More" aria-describedby="m{m.id}-by" tabindex={tool === 3 ? 0 : -1} aria-haspopup="menu" bind:this={moreEl} onclick={() => (menu = { mounted: true, open: !menu.open })}><Icon d={I.more} /></button>
    </div>
  {/if}
</article>

{#if menu.mounted}
  <Menu anchor={moreEl} open={menu.open} onclose={() => (menu.open = false)} label="Message actions">
    {#if mine}<MenuItem icon={I.edit} onselect={() => startEditing()}>Edit</MenuItem>{/if}
    <MenuItem icon={I.copy} onselect={() => copy(m.body)}>Copy text</MenuItem>
    <MenuItem icon={I.link} onselect={() => copy(`${location.origin}/r/${m.room_id}#m${m.id}`)}>Copy link</MenuItem>
    {#if mine || isMod}<MenuItem icon={I.trash} danger onselect={() => (confirm = { mounted: true, open: true })}>Delete</MenuItem>{/if}
  </Menu>
{/if}

{#if picker.mounted}
  <Popover anchor={reactEl} open={picker.open} onclose={() => (picker.open = false)} label="Add reaction"
    onopen={(el) => el.querySelector<HTMLElement>('button')?.focus()}>
    <div class="flex items-center gap-1">
      {#each EMOJIS as e (e)}
        <button type="button" class="pick" aria-label="React {e}" aria-pressed={has(e)} onclick={() => pick(e)}>{e}</button>
      {/each}
    </div>
    <form class="mt-1 flex gap-1" onsubmit={pickTyped}>
      <input class="control h-8 min-w-0 flex-1 px-2" aria-label="Any symbol" placeholder="Any symbol" maxlength="16" bind:value={symbol} />
      <Button size="sm" type="submit">Add</Button>
    </form>
  </Popover>
{/if}

{#if confirm.mounted}
  <Dialog open={confirm.open} title="Delete this message?" onclose={() => (confirm.open = false)}>
    <p>It is removed for everyone in the room. This cannot be undone.</p>
    {#snippet footer()}
      <Button variant="ghost" onclick={() => (confirm.open = false)}>Cancel</Button>
      <Button variant="danger" onclick={() => { confirm.open = false; void act(() => remove(m.id)); }}>Delete</Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .msg {
    position: relative;
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr);
    column-gap: 12px;
    padding: 4px 16px;
    font-size: 14px;
    line-height: 20px;
    /* Rows off screen skip layout and paint; the last size seen is kept (chat research item 13). */
    content-visibility: auto;
    contain-intrinsic-size: auto 56px;
    /* The highlight (hover, keyboard focus, editing, a mention) fades: in over --dur-2 (the rules below), out over
       --dur-3, so moving between rows crossfades. Paint only; the focus ring itself has no transition. */
    transition: background-color var(--dur-3) var(--ease-out);
  }
  .msg.head { margin-top: 8px; padding-top: 8px; }
  .msg:focus-visible { outline-offset: -2px; } /* inside the list, which clips the sides */
  /* A mention of me: a 2 px amber bar and a faint amber wash. */
  .msg.mentioned { background: color-mix(in oklab, var(--accent-subtle) 55%, transparent); box-shadow: inset 2px 0 0 var(--accent); }
  .msg:hover, .msg:focus-within, .msg.editing { background: var(--bg-hover); transition-duration: var(--dur-2); }
  .msg.editing { scroll-margin-bottom: 16px; } /* the list's bottom padding: an edit of the last row ends at the bottom */
  .msg.mentioned:hover, .msg.mentioned:focus-within { background: var(--accent-subtle); }
  .quote, .content { grid-column: 2; }
  .gutter { grid-column: 1; }
  .quote + .gutter { grid-row: 2; }
  /* Tertiary text: --fg-3 on the app background, --fg-2 once the row has a hover, selected or amber background. */
  .quiet { color: var(--fg-3); transition: color var(--dur-3) var(--ease-out); }
  .msg:hover .quiet, .msg:focus-within .quiet, .msg.mentioned .quiet, .msg.editing .quiet { color: var(--fg-2); }
  .quote {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 6px;
    margin-bottom: 2px;
    border-radius: var(--radius-sm);
    font-size: 12px;
    line-height: 16px;
    text-align: left;
  }
  .quote:hover { color: var(--fg); }
  .meta { display: flex; align-items: baseline; gap: 8px; }
  .meta :global(svg) { align-self: center; width: 12px; height: 12px; }
  .gtime {
    justify-self: end;
    margin-top: 2px;
    color: var(--fg-2);
    font-size: 12px;
    line-height: 16px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    opacity: 0;
    transition: opacity var(--dur-3) var(--ease-out);
  }
  .msg:hover :is(.gtime, .tools, .quiet), .msg:focus-within :is(.gtime, .tools, .quiet) { transition-duration: var(--dur-2); }
  .msg:hover .gtime, .msg:focus-within .gtime { opacity: 1; }
  .clamp {
    max-height: 400px; /* 20 lines */
    overflow: hidden;
    mask-image: linear-gradient(to bottom, var(--bg-app) 75%, transparent); /* a mask: only the alpha counts */
  }
  /* The (edited) and Pinned note ends the text's last line instead of taking a line of its own. */
  .body > :global(div) { display: inline; }
  .note { margin-left: 4px; font-size: 12px; line-height: 16px; white-space: nowrap; }
  .more-link { justify-self: start; color: var(--fg-2); font-size: 12px; line-height: 16px; }
  .more-link:hover { color: var(--fg); text-decoration: underline; text-underline-offset: 2px; }
  .pill {
    display: inline-flex;
    height: 24px;
    align-items: center;
    gap: 4px;
    padding: 0 8px;
    border: 1px solid var(--line-subtle);
    border-radius: 999px;
    background: var(--bg-element);
    color: var(--fg-2);
    font-size: 12px;
    line-height: 16px;
  }
  .pill:hover { border-color: var(--line-strong); color: var(--fg); }
  .pill.on { border-color: var(--accent-line); background: var(--accent-subtle); color: var(--accent-fg); }
  /* The toolbar sits inside the row's box: content-visibility clips paint to it. 28 px fits a one-line row. */
  .tools {
    position: absolute;
    top: 0;
    right: 16px;
    display: flex;
    padding: 1px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-raised);
    opacity: 0;
    transition: opacity var(--dur-3) var(--ease-out);
  }
  .msg.head .tools { top: 4px; }
  .msg:hover .tools, .msg:focus-within .tools, .tools:has(:global([aria-expanded='true'])) { opacity: 1; }
  .tb {
    display: inline-flex;
    width: 24px;
    height: 24px;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    color: var(--fg-2);
  }
  .tb:hover { background: var(--bg-hover); color: var(--fg); }
  .tb:focus-visible { outline-offset: -2px; }
  .tb:global([aria-expanded='true']) { background: var(--bg-selected); color: var(--fg); }
  .pick { width: 32px; height: 32px; border-radius: var(--radius); font-size: 16px; }
  .pick:hover, .pick[aria-pressed='true'] { background: var(--bg-hover); }
  @media (prefers-reduced-motion: reduce) {
    .msg, .tools, .gtime, .quiet { transition: none; }
  }
</style>
