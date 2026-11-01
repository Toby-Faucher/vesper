<script lang="ts">
  // The message box at the bottom of a room: an auto-growing textarea (Enter sends, Shift+Enter adds a line, Up in an
  // empty box edits your last message), the reply bar, attachments, the @ mention list and the formatting help. One
  // 16 px line above the box holds, in order: the offline line, a send error, the too-long line, or who is typing, so
  // nothing moves when one appears. Drafts live in chat.drafts by room, in memory only. Names are untrusted: text nodes.
  // Live preview: the textarea's own text is transparent and a mirror layer under it draws the same characters, styled by
  // decorate() (markers dimmed, content styled). The textarea stays the real control, so IME, undo, paste, the caret and
  // selection are the browser's. The layer may only change colour, background and decoration, never glyph widths.
  import { tick, untrack } from 'svelte';
  import { ApiFailure, errorText } from '../lib/api';
  import { chat, lastOwn, send, typers } from '../lib/chat.svelte';
  import type { Message } from '../lib/gen/Message';
  import { decorate, parse } from '../lib/markdown';
  import { session } from '../lib/session.svelte';
  import * as I from '../lib/ui/icons';
  import { isSendKey } from '../lib/ui/keys';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import { loadUsers } from '../lib/ui/users';
  import { mentionQuery, mentionable, plain, typingLine } from '../lib/ui/timeline';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  import Popover from '../lib/ui/c/Popover.svelte';
  import { tray } from '../lib/ui/tray.svelte';
  import UploadTray from './UploadTray.svelte';

  let { roomId, replyTo = $bindable(null) }: { roomId: number; replyTo?: Message | null } = $props();

  const TYPING_EVERY_MS = 3000; // the server takes one typing event per 3 s
  const MAX = 4000; // the server's limit, in characters
  const COUNT_FROM = 3600; // the counter shows from here
  const PASTE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']; // the types the server shows inline
  const uid = $props.id();

  let el = $state<HTMLTextAreaElement>();
  let mirror = $state<HTMLDivElement>();
  let helpEl = $state<HTMLButtonElement | null>(null);
  let help = $state({ mounted: false, open: false });
  let error = $state('');
  let sending = $state(false);
  let lastTyping = 0;
  let now = $state(Date.now());
  let mention = $state<{ start: number; q: string } | null>(null);
  let active = $state(0);
  let dismissed = -1; // the @ that Esc closed: its list stays shut until the caret leaves it
  let seenRefocus = chat.refocus;

  const myId = $derived(session.state.me?.user.user_id ?? '');
  const room = $derived(session.state.rooms.find((r) => r.id === roomId));
  const draft = $derived(chat.drafts[roomId] ?? '');
  const name = (id: string) => session.state.users[id]?.display_name ?? 'Someone';
  const runs = $derived(decorate(draft));
  const count = $derived([...draft.trim().normalize('NFC')].length);
  const over = $derived(count - MAX);
  const online = $derived(session.online);
  const files = $derived(tray.of(roomId));
  const busy = $derived(tray.busy(roomId));
  const done = $derived(tray.done(roomId));
  // Uploads still running in rooms you left, so a room change never hides them.
  const away = $derived(tray.elsewhere(roomId).map((id) => session.state.rooms.find((r) => r.id === id)?.name ?? 'another room'));
  // Polite, and only when the count changes: not per progress step. A failure has its own alert in the row.
  const trayLine = $derived(busy ? UI_MESSAGES.uploading(files.filter((i) => i.state === 'queued' || i.state === 'uploading').length) : done.length ? UI_MESSAGES.filesReady(done.length) : '');
  const canSend = $derived(online && over <= 0 && !busy && !sending && (!!draft.trim() || done.length > 0));
  const people = $derived(mention && room ? mentionable(mention.q, Object.values(session.state.users), room, myId) : []);
  const listOpen = $derived(people.length > 0);
  const typingIds = $derived(typers(chat.state, roomId, now).filter((id) => id !== myId));
  const typing = $derived(typingLine(typingIds.map(name)));
  const notice = $derived(!online ? UI_MESSAGES.composerOffline : error || (over > 0 ? UI_MESSAGES.tooLong(over) : ''));

  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });

  // A new room: the reply and the mention list belong to the old one. The draft and the files are kept by room.
  $effect(() => {
    void roomId;
    untrack(() => {
      replyTo = null;
      error = '';
      mention = null;
    });
  });

  // Someone who signed up after this tab connected is not in hello's list: ask once when a name is needed.
  $effect(() => {
    if (mention || typingIds.some((id) => !session.state.users[id])) loadUsers().catch(() => {});
  });

  // 1 to 10 lines, then it scrolls.
  $effect(() => {
    void draft;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 10 * 20 + 12)}px`; // 20 px lines plus 12 px padding
    sync();
  });

  const sync = () => mirror && el && (mirror.scrollTop = el.scrollTop);

  // An edit started from here (Up) ended: focus comes back.
  $effect(() => {
    const n = chat.refocus;
    untrack(() => {
      if (n !== seenRefocus) el?.focus();
      seenRefocus = n;
    });
  });

  export function focus() {
    el?.focus();
  }

  function setDraft(v: string) {
    chat.drafts[roomId] = v;
  }

  async function submit(e?: Event) {
    e?.preventDefault();
    if (!canSend) return;
    const room = roomId;
    const text = draft;
    const taken = done; // a file that finishes during the send stays for the next one
    const atts = taken.map((i) => i.attachment!);
    const body = text.trim() || atts.map((a) => a.name).join(', '); // a body is required
    sending = true;
    error = '';
    try {
      await send(room, body, replyTo?.id ?? null, atts.map((a) => a.id));
      if (chat.drafts[room] === text) chat.drafts[room] = ''; // text typed while it was sending stays
      replyTo = null;
      tray.sent(taken.map((i) => i.key));
      mention = null;
      chat.latest++; // follow your own message, also from an older slice after a jump
    } catch (err) {
      // The draft is kept, so a retry is one key away.
      error = err instanceof ApiFailure && err.code === 'conflict' ? UI_MESSAGES.attachFailed : errorText(err);
    } finally {
      sending = false;
    }
  }

  function findMention() {
    if (!el) return;
    const m = el.selectionStart === el.selectionEnd ? mentionQuery(el.value, el.selectionStart) : null;
    if (m?.start !== mention?.start) active = 0;
    if (m && m.start === dismissed) {
      mention = null;
      return;
    }
    dismissed = -1;
    mention = m;
  }

  async function pick(username: string) {
    if (!mention || !el) return;
    const caret = el.selectionStart;
    const ins = `@${username} `;
    setDraft(draft.slice(0, mention.start) + ins + draft.slice(caret));
    const at = mention.start + ins.length;
    mention = null;
    await tick();
    el.setSelectionRange(at, at);
    el.focus();
  }

  function onKey(e: KeyboardEvent) {
    if (listOpen && !e.isComposing) {
      const n = people.length;
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        active = (active + (e.key === 'ArrowDown' ? 1 : n - 1)) % n;
        return;
      }
      if ((e.key === 'Enter' && !e.shiftKey && e.keyCode !== 229) || (e.key === 'Tab' && !e.shiftKey)) {
        e.preventDefault();
        void pick(people[Math.min(active, n - 1)].username);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        dismissed = mention!.start;
        mention = null;
        return;
      }
    }
    if (e.key === 'Escape' && replyTo) {
      e.preventDefault();
      e.stopPropagation();
      replyTo = null;
    } else if (e.key === 'ArrowUp' && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey && draft === '' && !files.length) {
      const m = lastOwn(roomId, myId);
      if (!m) return;
      e.preventDefault();
      chat.edit = m.id;
    } else if (isSendKey(e)) {
      e.preventDefault();
      void submit();
    }
  }

  function onInput() {
    findMention();
    const t = Date.now();
    if (error) error = '';
    if (!draft.trim() || t - lastTyping < TYPING_EVERY_MS || !online) return;
    lastTyping = t;
    session.sig?.send({ type: 'typing', room_id: roomId });
  }

  function attach(files: File[]) {
    if (files.length) tray.add(roomId, files);
  }

  function onPaste(e: ClipboardEvent) {
    const files = [...(e.clipboardData?.files ?? [])].filter((f) => PASTE_TYPES.includes(f.type));
    if (!files.length) return;
    e.preventDefault();
    attach(files);
  }

  function cancelReply() {
    replyTo = null;
    el?.focus();
  }
</script>

<div class="shrink-0 px-4 pb-4">
  <div class="slot">
    {#if online && error}<p class="truncate text-danger" role="alert" title={error}>{error}</p>{/if}
    <!-- Always rendered, so a new text is announced: the offline and too-long lines, else who is typing (that changes when
         someone starts or stops, not per key press). -->
    <p class="truncate" class:hidden={online && !!error} class:text-danger={over > 0} class:text-fg-2={!online && over <= 0}
      class:text-fg-3={online && over <= 0} role="status" title={notice || undefined}>{online && error ? '' : notice || typing}</p>
  </div>

  <span class="sr-only" role="status">{trayLine}</span>
  <form class="box" onsubmit={submit}>
    {#if replyTo}
      <div class="bar">
        <Icon d={I.reply} size={16} />
        <p class="min-w-0 flex-1 truncate">
          Replying to <span class="font-medium text-fg">{name(replyTo.author_id)}</span>
          <span class="ml-2 text-fg-2">{plain(parse(replyTo.body)) || 'a message with a file'}</span>
        </p>
        <button type="button" class="mini-x" aria-label="Cancel reply" onclick={cancelReply}><Icon d={I.x} size={16} /></button>
      </div>
    {/if}
    {#if away.length}
      <p class="away"><Icon d={I.upload} size={16} /><span class="truncate">Uploading in {away.join(', ')}</span></p>
    {/if}
    {#if files.length}<UploadTray {tray} items={files} />{/if}

    <div class="row">
      <!-- The native picker stays in the page for the keyboard (Tab reaches it, Space opens it), drawn as the icon. -->
      <label class="attach" data-tip="Attach a file" data-tip-up>
        <Icon d={I.paperclip} />
        <span class="sr-only">Attach a file</span>
        <input type="file" multiple class="sr-only"
          onchange={(e) => { attach([...(e.currentTarget.files ?? [])]); e.currentTarget.value = ''; }} />
      </label>
      <div class="field">
      <!-- The trailing space keeps a last empty line as tall as the textarea's. -->
      <div class="mirror" aria-hidden="true" bind:this={mirror}>{#each runs as r}<span class={r.k}>{r.v}</span>{/each}{' '}</div>
      <textarea bind:this={el} rows="1" aria-label="Message" placeholder="Message {room?.name ?? ''}"
        aria-autocomplete="list" aria-controls={listOpen ? `${uid}-people` : undefined}
        aria-activedescendant={listOpen ? `${uid}-p${active}` : undefined}
        bind:value={() => draft, setDraft} onkeydown={onKey} oninput={onInput} onpaste={onPaste} onclick={findMention}
        onkeyup={(e) => (e.key.startsWith('Arrow') || e.key === 'Home' || e.key === 'End') && !listOpen && findMention()}
        onblur={() => (mention = null)} onscroll={sync}></textarea>
      </div>
      {#if count >= COUNT_FROM}
        <span class="count" class:over={over > 0}>{count}/{MAX}</span>
      {/if}
      <IconButton icon={I.help} label="Formatting help" bind:el={helpEl} onclick={() => (help = { mounted: true, open: !help.open })} />
      <button type="submit" class="send" aria-label="Send" aria-disabled={!canSend} aria-busy={sending || undefined} data-tip={busy ? UI_MESSAGES.waitUploads : 'Send'} data-tip-up>
        <Icon d={I.send} />
      </button>
    </div>

    {#if listOpen}
      <ul class="people" id="{uid}-people" role="listbox" aria-label="People">
        {#each people as p, i (p.user_id)}
          <!-- The textarea keeps focus and the keys (aria-activedescendant); a click picks without taking it. -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li id="{uid}-p{i}" role="option" aria-selected={i === active} class:on={i === active}
            onpointerdown={(e) => e.preventDefault()} onclick={() => pick(p.username)} onpointermove={() => (active = i)}>
            <Avatar user={p} size={20} />
            <span class="truncate text-fg">{p.display_name}</span>
            <span class="truncate text-fg-2">@{p.username}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </form>
</div>

{#if help.mounted}
  <Popover anchor={helpEl} open={help.open} side="top" label="Formatting" onclose={() => (help.open = false)}>
    <div class="help">
      <p class="mb-1 font-medium text-fg">Formatting</p>
      <dl>
        <dt><code>**bold**</code></dt><dd><strong class="font-medium text-fg">bold</strong></dd>
        <dt><code>*italic*</code></dt><dd><em>italic</em></dd>
        <dt><code>~~strike~~</code></dt><dd><s>strike</s></dd>
        <dt><code>`code`</code></dt><dd><code>code</code></dd>
        <dt><code>```rust</code> lines <code>```</code></dt><dd>code block</dd>
        <dt><code># title</code></dt><dd>heading</dd>
        <dt><code>&gt; text</code></dt><dd>quote</dd>
        <dt><code>- item</code>, <code>1. item</code></dt><dd>list</dd>
        <dt><code>---</code></dt><dd>rule</dd>
        <dt><code>https://...</code></dt><dd>link</dd>
        <dt><code>@name</code></dt><dd>mention</dd>
      </dl>
      <p class="mt-2 text-fg-2">HTML and images show as typed. Enter sends, Shift+Enter adds a line, Up edits your last message.</p>
    </div>
  </Popover>
{/if}

<style>
  .slot { position: relative; height: 16px; margin: 0 4px 4px; font-size: 12px; line-height: 16px; }
  .box {
    position: relative;
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    background: var(--bg-element);
    transition: border-color var(--dur-1) var(--ease-out);
  }
  .box:hover { border-color: var(--line-strong); }
  /* The box carries the textarea's focus ring. */
  .box:has(textarea:focus-visible) { border-color: var(--focus); }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 4px 0 12px;
    border-bottom: 1px solid var(--line-subtle);
    color: var(--fg-2);
    font-size: 12px;
    line-height: 16px;
  }
  .away { display: flex; align-items: center; gap: 8px; height: 32px; padding: 0 12px; border-bottom: 1px solid var(--line-subtle); color: var(--fg-2); font-size: 12px; line-height: 16px; }
  .row { display: flex; align-items: flex-end; gap: 4px; padding: 4px; }
  .field { position: relative; flex: 1; min-width: 0; }
  textarea, .mirror {
    padding: 6px 4px;
    font-size: 14px;
    line-height: 20px;
    white-space: pre-wrap;
    overflow-wrap: break-word;
    overflow-y: auto;
  }
  textarea {
    position: relative;
    display: block;
    width: 100%;
    resize: none;
    background: transparent;
    color: transparent;
    caret-color: var(--fg);
  }
  textarea::selection { background: var(--bg-selected); color: transparent; }
  /* Same box, same scroll range, so the same lines wrap; its scrollbar is hidden under the textarea's. */
  .mirror { position: absolute; inset: 0; color: var(--fg); pointer-events: none; scrollbar-color: transparent transparent; }
  .mirror :global(.mk) { color: var(--fg-3); }
  /* Bold by a stroke, not a weight: a heavier face is wider and would pull the text away from the caret. */
  .mirror :global(.b), .mirror :global(.h) { -webkit-text-stroke: 0.04em currentColor; }
  .mirror :global(.h) { color: var(--fg); }
  /* shortcut: italic is not slanted here, a real italic face has other widths; it shows by its dimmed markers only. */
  .mirror :global(.s) { text-decoration: line-through; }
  .mirror :global(.code), .mirror :global(.pre) { border-radius: var(--radius-sm); background: var(--bg-selected); }
  .mirror :global(.q) { color: var(--fg-2); }
  .mirror :global(.link) { text-decoration: underline; text-underline-offset: 2px; }
  /* Forced colours repaint the textarea's text: then it is the only layer. */
  @media (forced-colors: active) {
    .mirror { display: none; }
    textarea { color: CanvasText; }
  }
  textarea::placeholder { color: var(--fg-3); }
  textarea:focus-visible { outline: none; } /* drawn on .box above */
  .attach, .send {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    color: var(--fg-2);
    transition: background-color var(--dur-1) var(--ease-out), color var(--dur-1) var(--ease-out);
  }
  .attach, .send { width: 32px; height: 32px; cursor: pointer; }
  .attach:hover { background: var(--bg-hover); color: var(--fg); }
  .attach:has(input:focus-visible) { outline: 2px solid var(--focus); outline-offset: 2px; }
  .send { color: var(--accent-fg); }
  .send:hover { background: var(--bg-hover); }
  .send[aria-disabled='true'] { cursor: default; color: var(--fg-disabled); background: none; }
  .count { padding: 0 4px; color: var(--fg-2); font-size: 12px; line-height: 32px; font-variant-numeric: tabular-nums; }
  .count.over { color: var(--danger-fg); }
  .people {
    position: absolute;
    bottom: calc(100% + 4px);
    left: 0;
    width: 320px;
    max-height: 296px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--line-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-raised);
    box-shadow: var(--shadow-float);
    font-size: 13px;
    line-height: 20px;
  }
  .people li {
    display: flex;
    height: 32px;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-radius: var(--radius);
    cursor: pointer;
  }
  .people li.on { background: var(--line); } /* as the switcher's keyboard row: --bg-selected is too faint on --bg-raised */
  .help { width: 288px; padding: 8px; }
  .help dl { display: grid; grid-template-columns: auto 1fr; gap: 4px 16px; align-items: baseline; }
  .help dd { color: var(--fg-2); }
  .help code { padding: 0 4px; border-radius: var(--radius-sm); background: var(--bg-element); font-family: var(--font-mono); font-size: 12px; }
  @media (prefers-reduced-motion: reduce) {
    .box, .attach, .send { transition: none; }
  }
</style>
