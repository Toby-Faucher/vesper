<script lang="ts">
  import * as I from '../../lib/ui/icons';
  // Dev-only: every primitive in lib/ui/c in its states. Elements with data-force get :hover or :focus-visible forced by
  // the screenshot scene `gallery` (Chrome DevTools protocol); in a normal browser they look like the default.
  import { SLOT_HEX } from '../../lib/ui/avatar';
  import Avatar from '../../lib/ui/c/Avatar.svelte';
  import Badge from '../../lib/ui/c/Badge.svelte';
  import Button from '../../lib/ui/c/Button.svelte';
  import Dialog from '../../lib/ui/c/Dialog.svelte';
  import EmptyState from '../../lib/ui/c/EmptyState.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import Icon from '../../lib/ui/c/Icon.svelte';
  import IconButton from '../../lib/ui/c/IconButton.svelte';
  import Kbd from '../../lib/ui/c/Kbd.svelte';
  import Mark from '../../lib/ui/c/Mark.svelte';
  import Menu from '../../lib/ui/c/Menu.svelte';
  import MenuItem from '../../lib/ui/c/MenuItem.svelte';
  import Popover from '../../lib/ui/c/Popover.svelte';
  import Range from '../../lib/ui/c/Range.svelte';
  import Select from '../../lib/ui/c/Select.svelte';
  import Speaking from '../../lib/ui/c/Speaking.svelte';
  import Status from '../../lib/ui/c/Status.svelte';
  import Switch from '../../lib/ui/c/Switch.svelte';
  import TextInput from '../../lib/ui/c/TextInput.svelte';

  const variants = [
    ['primary', 'Join voice'],
    ['secondary', 'Copy link'],
    ['ghost', 'Skip for now'],
    ['danger', 'Leave'],
  ] as const;
  const states = ['Default', 'Hover', 'Focus', 'Disabled', 'Loading', 'Small'];
  const ibStates: [string, Record<string, unknown>][] = [
    ['Default', {}], ['Hover', { 'data-force': 'hover' }], ['Focus', { 'data-force': 'focus-visible' }], ['Disabled', { disabled: true }],
  ];
  const force = (s: string) => (s === 'Hover' ? 'hover' : s === 'Focus' ? 'focus-visible' : undefined);

  const names = ['alice', 'bob', 'carol', 'Émile Zola', 'Mina Park', 'toby', 'Jo', '🎸 Sam'];
  const users = names.map((display_name, i) => ({ user_id: `${i}`.padStart(32, 'a'), display_name, accent: SLOT_HEX[i] }));
  const hashed = { user_id: '3f2a9c0b7d1e4a5b6c7d8e9f0a1b2c3d', display_name: 'dana', accent: 'server default' } // not a swatch: hashed;
  const presence = [
    { u: users[0], speaking: true, muted: false, deafened: false },
    { u: users[1], speaking: false, muted: true, deafened: false },
    { u: users[2], speaking: false, muted: false, deafened: true },
    { u: hashed, speaking: false, muted: false, deafened: false },
  ];

  let name = $state('toby');
  let room = $state('');
  let mode = $state('deepfilter');
  let volume = $state(100);
  let ptt = $state(true);
  let sounds = $state(false);
  let muted = $state(true);
  let last = $state('Nothing yet');

  let dialogOpen = $state(false);
  let dialogCloses = $state(0); // read by the gallery scene: onclose runs once per close
  // The gallery scene removes the open dialog (as a re-key or a sign out does) to check that focus still goes back.
  let dialogMounted = $state(true);
  (window as unknown as { __unmountDialog: () => void }).__unmountDialog = () => (dialogMounted = false);
  let confirmOpen = $state(false);
  let menuOpen = $state(false);
  let innerMenuOpen = $state(false);
  let cardOpen = $state(false);
  let menuBtn = $state<HTMLButtonElement | null>(null);
  let innerBtn = $state<HTMLButtonElement | null>(null);
  let cardBtn = $state<HTMLButtonElement | null>(null);
  const did = (what: string) => () => (last = what);
</script>

<div class="space-y-10">
  <section class="space-y-3">
    <h2 class="text-sm font-medium text-fg">Buttons</h2>
    <div class="grid grid-cols-[96px_repeat(6,max-content)] items-center gap-x-6 gap-y-3">
      <span></span>
      {#each states as s (s)}<span class="text-xs text-fg-3">{s}</span>{/each}
      {#each variants as [v, text] (v)}
        <span class="font-mono text-xs text-fg-3">{v}</span>
        {#each states as s (s)}
          <Button variant={v} size={s === 'Small' ? 'sm' : 'md'} disabled={s === 'Disabled'} loading={s === 'Loading'} data-force={force(s)}>{text}</Button>
        {/each}
      {/each}
    </div>
  </section>

  <section class="space-y-3">
    <h2 class="text-sm font-medium text-fg">Icon buttons</h2>
    <div class="flex gap-6 pb-6">
      {#each ibStates as [s, extra] (s)}
        <div class="flex w-16 flex-col items-center gap-2">
          <span class="text-xs text-fg-3">{s}</span>
          <IconButton icon={I.settings} label="Settings ({s})" {...extra} />
        </div>
      {/each}
      <div class="flex w-16 flex-col items-center gap-2">
        <span class="text-xs text-fg-3">Pressed</span>
        <IconButton icon={muted ? I.micOff : I.mic} label="Mute" pressed={muted} onclick={() => (muted = !muted)} />
      </div>
      <div class="flex w-16 flex-col items-center gap-2">
        <span class="text-xs text-fg-3">Tooltip</span>
        <IconButton icon={I.search} label="Search messages" tip data-force="hover" />
      </div>
    </div>
    <ul class="grid grid-cols-8 gap-2">
      {#each Object.entries(I).filter((e): e is [string, I.IconPath] => typeof e[1] === 'string') as [n, d] (n)}
        <li class="flex h-8 items-center gap-2 text-xs text-fg-2"><Icon {d} /> {n}</li>
      {/each}
    </ul>
  </section>

  <section class="space-y-3">
    <h2 class="text-sm font-medium text-fg">Fields</h2>
    <div class="grid grid-cols-3 gap-x-8 gap-y-6">
      <Field label="Display name" help="Shown to everyone in the group."><TextInput bind:value={name} /></Field>
      <Field label="Room name" error="Pick a name with 1 to 32 characters."><TextInput bind:value={room} placeholder="music" /></Field>
      <Field label="Invite code" help="Ask the owner for a new link."><TextInput value="Used up" disabled /></Field>
      <Field label="Focused"><TextInput value="Typing here" data-force="focus-visible" /></Field>
      <Field label="Noise suppression" help="deepfilter: the best filter, runs in this tab.">
        <Select bind:value={mode}>
          <option value="deepfilter">deepfilter</option>
          <option value="browser">browser</option>
          <option value="off">off</option>
        </Select>
      </Field>
      <Field label="Volume" help="{volume} %, above 100 % can clip."><Range bind:value={volume} max={200} /></Field>
      <div class="flex flex-col">
        <Switch label="Push to talk" bind:checked={ptt} />
        <Switch label="Join and leave sounds" bind:checked={sounds} />
      </div>
      <div class="flex flex-col">
        <Switch label="Focused" checked data-force="focus-visible" />
        <Switch label="Disabled" disabled />
      </div>
      <Field label="Disabled select">
        <Select disabled value="browser"><option value="browser">browser</option></Select>
      </Field>
    </div>
  </section>

  <section class="space-y-3">
    <h2 class="text-sm font-medium text-fg">Avatars and badges</h2>
    <div class="flex gap-8">
      <div class="w-62 rounded-lg bg-sidebar p-2">
        <div class="flex h-7 items-center gap-2 rounded-sm px-2 font-medium text-fg">
          <span class="flex-1">general</span>
          <Badge kind="count" n={3} /><Badge kind="mention" n={1} />
        </div>
        <ul>
          {#each presence as p (p.u.user_id)}
            <li class="flex h-7 items-center gap-2 pr-2 pl-6 text-fg-2">
              <Avatar user={p.u} size={20} speaking={p.speaking} muted={p.muted} deafened={p.deafened} />
              <span class="truncate">{p.u.display_name}</span>
              {#if p.speaking}<Speaking />{/if}
              {#if p.deafened}<span class="text-danger"><Icon d={I.headphonesOff} /></span><span class="sr-only">deafened</span>
              {:else if p.muted}<span class="text-danger"><Icon d={I.micOff} /></span><span class="sr-only">muted</span>{/if}
            </li>
          {/each}
        </ul>
        <div class="flex h-7 items-center gap-2 rounded-sm px-2 text-fg-2">
          <span class="flex-1">music</span><Badge kind="count" n={120} />
        </div>
        <div class="flex h-7 items-center gap-2 rounded-sm px-2 text-fg-2">
          <span class="flex-1">lobby</span><Badge kind="mention" n={12} />
        </div>
      </div>
      <div class="space-y-4 rounded-lg bg-sidebar p-4">
        <ul class="flex gap-4">
          {#each users as u (u.user_id)}
            <li class="flex w-16 flex-col items-center gap-1">
              <Avatar user={u} size={32} />
              <span class="w-full truncate text-center text-xs text-fg-3">{u.display_name}</span>
            </li>
          {/each}
        </ul>
        <div class="grid grid-cols-[48px_repeat(4,64px)] items-center gap-y-4">
          <span></span>
          {#each ['Idle', 'Speaking', 'Muted', 'Deafened'] as s (s)}<span class="text-xs text-fg-3">{s}</span>{/each}
          {#each [20, 32, 40] as const as size (size)}
            <span class="font-mono text-xs text-fg-3">{size}</span>
            <span><Avatar user={users[5]} {size} /></span>
            <span><Avatar user={users[5]} {size} speaking /></span>
            <span class="flex items-center gap-1"><Avatar user={users[5]} {size} muted />{#if size === 20}<span class="text-danger"><Icon d={I.micOff} /></span>{/if}</span>
            <span class="flex items-center gap-1"><Avatar user={users[5]} {size} deafened />{#if size === 20}<span class="text-danger"><Icon d={I.headphonesOff} /></span>{/if}</span>
          {/each}
        </div>
      </div>
    </div>
  </section>

  <section class="grid grid-cols-2 gap-8">
    <div class="space-y-3">
      <h2 class="text-sm font-medium text-fg">Status lines</h2>
      <Status kind="info">Noise suppression: browser (the denoiser did not load)</Status>
      <Status kind="warn">Weak connection: 4 % loss</Status>
      <Status kind="error">Microphone blocked. Allow it in the address bar.</Status>
      <Status kind="ok">Invite link copied.</Status>
    </div>
    <div class="space-y-3">
      <h2 class="text-sm font-medium text-fg">Empty state, keys and the mark</h2>
      <div class="rounded-lg border border-subtle">
        <EmptyState title="Nobody is in voice." body="Join to start. Others see you here as soon as you do.">
          {#snippet action()}<Button>Join voice</Button>{/snippet}
        </EmptyState>
      </div>
      <div class="flex items-center gap-8">
        <span class="flex items-center gap-1 text-xs text-fg-3"><Kbd>Ctrl</Kbd><Kbd>K</Kbd> to switch rooms</span>
        <span class="flex items-center gap-2 text-sm font-medium"><Mark /> Vesper</span>
        <Mark size={32} />
      </div>
    </div>
  </section>

  <section class="space-y-3">
    <h2 class="text-sm font-medium text-fg">Layers</h2>
    <div class="flex items-center gap-3">
      <Button aria-haspopup="dialog" aria-controls="invite-dialog" onclick={() => (dialogOpen = true)}>Create invite</Button>
      <IconButton bind:el={menuBtn} icon={I.more} label="Message actions" onclick={() => (menuOpen = !menuOpen)} />
      <Button bind:el={cardBtn} variant="ghost" onclick={() => (cardOpen = !cardOpen)}>Show person card</Button>
      <p role="status" class="text-xs text-fg-3" data-closes={dialogCloses}>Last action: {last}</p>
    </div>
  </section>
</div>

<Menu anchor={menuBtn} open={menuOpen} onclose={() => (menuOpen = false)} label="Message actions">
  <MenuItem icon={I.reply} onselect={did('Reply')}>Reply</MenuItem>
  <MenuItem icon={I.edit} onselect={did('Edit')}>Edit</MenuItem>
  <MenuItem icon={I.pin} onselect={did('Pin')}>Pin</MenuItem>
  <MenuItem icon={I.copy} onselect={did('Copy link')}>Copy link</MenuItem>
  <MenuItem icon={I.smile} disabled onselect={did('React')}>React</MenuItem>
  <MenuItem icon={I.trash} danger onselect={did('Delete')}>Delete</MenuItem>
</Menu>

<Popover anchor={cardBtn} open={cardOpen} onclose={() => (cardOpen = false)} label="bob">
  <div class="w-64 space-y-3 p-2">
    <div class="flex items-center gap-3">
      <Avatar user={users[1]} size={40} />
      <div>
        <p class="font-medium text-fg">bob</p>
        <p class="text-xs text-fg-3">Member</p>
      </div>
    </div>
    <Field label="Volume"><Range bind:value={volume} max={200} /></Field>
  </div>
</Popover>

{#if dialogMounted}
<Dialog id="invite-dialog" open={dialogOpen} title="Create an invite" onclose={() => { dialogOpen = false; dialogCloses++; }}>
  <div class="space-y-4">
    <p>The link works once and expires after the time you pick.</p>
    <Field label="Role">
      <Select value="member"><option value="member">Member</option><option value="moderator">Moderator</option></Select>
    </Field>
    <div class="flex items-end gap-2">
      <div class="flex-1"><Field label="Expires after (hours)"><TextInput value="24" inputmode="numeric" /></Field></div>
      <IconButton bind:el={innerBtn} icon={I.chevronDown} label="Presets" onclick={() => (innerMenuOpen = !innerMenuOpen)} />
    </div>
    <!-- -ml-2: the label lines up with the fields above, the ghost padding hangs into the margin. -->
    <div class="-ml-2">
      <Button variant="ghost" size="sm" aria-haspopup="dialog" aria-controls="revoke-dialog" onclick={() => (confirmOpen = true)}>Revoke all links</Button>
    </div>
  </div>
  <!-- Inside the dialog: a modal dialog makes everything outside it inert, popovers included. -->
  <Menu anchor={innerBtn} open={innerMenuOpen} onclose={() => (innerMenuOpen = false)} label="Presets">
    <MenuItem onselect={did('1 hour')}>1 hour</MenuItem>
    <MenuItem onselect={did('24 hours')}>24 hours</MenuItem>
    <MenuItem onselect={did('7 days')}>7 days</MenuItem>
  </Menu>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (dialogOpen = false)}>Cancel</Button>
    <Button variant="primary" onclick={() => { last = 'Create link'; dialogOpen = false; }}>Create link</Button>
  {/snippet}
</Dialog>
{/if}

<!-- A dialog on top of a dialog: Esc closes only this one and focus goes back to "Revoke all links". -->
<Dialog id="revoke-dialog" open={confirmOpen} title="Revoke all invite links?" onclose={() => (confirmOpen = false)}>
  <p>Links already sent stop working. People who joined stay.</p>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (confirmOpen = false)}>Cancel</Button>
    <Button variant="danger" onclick={() => { last = 'Revoke'; confirmOpen = false; }}>Revoke</Button>
  {/snippet}
</Dialog>
