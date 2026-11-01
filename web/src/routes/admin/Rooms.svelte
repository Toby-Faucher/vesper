<script lang="ts">
  import * as I from '../../lib/ui/icons';
  import { tick } from 'svelte';
  import { ApiFailure, api, errorText } from '../../lib/api';
  import { session } from '../../lib/session.svelte';
  import { UI_MESSAGES as M } from '../../lib/ui/messages';
  import { byRole, moveRoom } from './admin';
  import Button from '../../lib/ui/c/Button.svelte';
  import Dialog from '../../lib/ui/c/Dialog.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import IconButton from '../../lib/ui/c/IconButton.svelte';
  import Switch from '../../lib/ui/c/Switch.svelte';
  import TextInput from '../../lib/ui/c/TextInput.svelte';
  import type { RoomInfo } from '../../lib/gen/RoomInfo';
  import type { UserInfo } from '../../lib/gen/UserInfo';
  // Admin > Rooms, the owner's (the server checks it on every route). The list is the session's, which rooms_changed keeps
  // current; after a change succeeds it is updated here at once too, so the row (and focus in it) does not wait for the event.
  // The server has no delete route: a room is hidden by restricting it.
  const rooms = $derived([...session.state.rooms].sort((a, b) => a.position - b.position));
  const put = (r: RoomInfo) => (session.state.rooms = session.state.rooms.some((x) => x.id === r.id) ? session.state.rooms.map((x) => (x.id === r.id ? r : x)) : [...session.state.rooms, r]);

  // A 400 on a name is the name rule; a 409 is a taken name (the table says so).
  const nameError = (e: unknown) => (e instanceof ApiFailure && e.code === 'bad_request' ? M.roomNameBad : errorText(e));
  const focusAlert = async (scope: ParentNode = document) => { await tick(); scope.querySelector<HTMLElement>('[role=alert]')?.focus(); };

  let newName = $state('');
  let creating = $state(false);
  let createError = $state('');
  let createdName = $state('');
  let form: HTMLFormElement;
  async function create(e: Event) {
    e.preventDefault();
    if (creating) return;
    creating = true;
    [createError, createdName] = ['', ''];
    try {
      const r = await api<RoomInfo>('POST', '/api/rooms', { name: newName });
      put(r);
      [newName, createdName] = ['', r.name];
    } catch (err) {
      createError = nameError(err);
      await focusAlert(form);
    } finally {
      creating = false;
    }
  }

  // One change at a time across the list; a second click while one runs is ignored.
  let busy = $state(0);
  let listError = $state('');
  let list: HTMLElement;
  async function run<T>(id: number, f: () => Promise<T>): Promise<T | undefined> {
    if (busy) return;
    busy = id;
    listError = '';
    try {
      return await f();
    } catch (e) {
      listError = errorText(e);
      await focusAlert(list);
    } finally {
      busy = 0;
    }
  }

  let editing = $state<{ id: number; value: string; error: string } | null>(null);
  async function rename() {
    if (!editing || busy) return;
    const ed = editing;
    busy = ed.id;
    ed.error = '';
    try {
      put(await api<RoomInfo>('PATCH', `/api/rooms/${ed.id}`, { name: ed.value }));
      editing = null;
      await tick();
      list.querySelector<HTMLElement>(`tr[data-room="${ed.id}"] [data-rename]`)?.focus();
    } catch (e) {
      ed.error = nameError(e); // focus stays in the field; the alert is read out and tied to it
    } finally {
      busy = 0;
    }
  }
  async function startRename(r: RoomInfo) {
    editing = { id: r.id, value: r.name, error: '' };
    await tick();
    list.querySelector<HTMLInputElement>(`tr[data-room="${r.id}"] input[type=text]`)?.select();
  }
  async function cancelRename() {
    const id = editing?.id;
    editing = null;
    await tick();
    list.querySelector<HTMLElement>(`tr[data-room="${id}"] [data-rename]`)?.focus();
  }

  // The whole order goes to the server; a 409 means the list changed meanwhile (a room added elsewhere): reload and say so.
  async function move(r: RoomInfo, by: -1 | 1) {
    const ids = rooms.map((x) => x.id);
    const next = moveRoom(ids, r.id, by);
    if (next === ids || busy) return;
    const had = document.activeElement as HTMLElement | null;
    const ok = await run(r.id, async () => {
      try {
        await api('PUT', '/api/rooms/order', { ids: next });
        return true;
      } catch (e) {
        if (!(e instanceof ApiFailure && e.status === 409)) throw e;
        session.state.rooms = await api<RoomInfo[]>('GET', '/api/rooms').catch(() => session.state.rooms);
        listError = M.roomOrderChanged;
        await focusAlert(list);
      }
    });
    if (!ok) return;
    session.state.rooms = session.state.rooms.map((x) => ({ ...x, position: next.indexOf(x.id) }));
    await tick();
    // The row moved with its buttons; a move button now at an end is disabled, so focus goes to its partner.
    if (had?.isConnected) (had.matches(':disabled') ? had.parentElement?.querySelector<HTMLElement>('[data-move]:enabled') : had)?.focus();
  }
  function rowKey(r: RoomInfo, e: KeyboardEvent) {
    if (!e.altKey || editing?.id === r.id || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
    e.preventDefault();
    void move(r, e.key === 'ArrowUp' ? -1 : 1);
  }

  async function restrict(r: RoomInfo, e: Event) {
    const box = e.currentTarget as HTMLInputElement;
    if (box.checked === r.restricted) return; // the change event sent below to put a failed switch back
    const fresh = await run(r.id, () => api<RoomInfo>('PATCH', `/api/rooms/${r.id}`, { restricted: box.checked }));
    if (fresh) return put(fresh);
    box.checked = r.restricted;
    box.dispatchEvent(new Event('change')); // so Switch's own binding agrees with what is shown
  }

  // Members: every account that is not banned, by role then name; a search field once the list is long.
  let picking = $state<RoomInfo | null>(null);
  let picked = $state<string[]>([]);
  let q = $state('');
  let pickError = $state('');
  let saving = $state(false);
  const people = $derived(Object.values(session.state.users).filter((u) => !u.banned).sort(byRole));
  const shown = $derived(people.filter((u) => !q.trim() || `${u.display_name} ${u.username}`.toLowerCase().includes(q.trim().toLowerCase())));
  function openMembers(r: RoomInfo) {
    [picking, picked, q, pickError] = [r, [...r.members], '', ''];
    // Fresh each time: no event announces a signup, so the session's map can miss people who joined since it loaded.
    api<UserInfo[]>('GET', '/api/users').then(
      (l) => (session.state.users = { ...session.state.users, ...Object.fromEntries(l.map((u) => [u.user_id, u])) }),
      () => {}, // the list from the session stands in
    );
  }
  async function saveMembers() {
    if (!picking || saving) return;
    saving = true;
    pickError = '';
    try {
      put(await api<RoomInfo>('PUT', `/api/rooms/${picking.id}/members`, { user_ids: picked }));
      picking = null;
    } catch (e) {
      pickError = errorText(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="sections">
  <section role="group" aria-labelledby="rooms-new">
    <h3 id="rooms-new" class="text-base font-medium text-fg">New room</h3>
    <form onsubmit={create} bind:this={form} class="flex max-w-120 flex-col gap-3">
      <Field label="Room name" help="1 to 32 characters.">
        <div class="flex gap-2">
          <div class="min-w-0 flex-1"><TextInput bind:value={newName} required maxlength={64} autocomplete="off" /></div>
          <Button type="submit" variant="primary" loading={creating}>Create room</Button>
        </div>
      </Field>
      {#if createError}
        <p role="alert" tabindex="-1" class="-mt-1 text-sm text-danger">{createError}</p>
      {:else}
        <p role="status" class="-mt-1 text-sm text-fg-2">{createdName ? `Created ${createdName}.` : ''}</p>
      {/if}
    </form>
  </section>

  <section role="group" aria-labelledby="rooms-list" bind:this={list}>
    <h3 id="rooms-list" class="text-base font-medium text-fg">All rooms</h3>
    <table class="tbl" aria-labelledby="rooms-list">
      <thead>
        <tr><th scope="col">Name</th><th scope="col" class="w-44">Restricted</th><th class="w-40"><span class="sr-only">Actions</span></th></tr>
      </thead>
      <tbody>
        {#each rooms as r, i (r.id)}
          <tr data-room={r.id} onkeydown={(e) => rowKey(r, e)}>
            <td>
              {#if editing?.id === r.id}
                <input type="text" class="control h-8 w-full px-2.5" bind:value={editing.value} aria-label="Name for {r.name}"
                  aria-invalid={!!editing.error} aria-describedby={editing.error ? `rename-${r.id}` : undefined}
                  onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void rename(); } else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); void cancelRename(); } }} />
                {#if editing.error}<p id="rename-{r.id}" role="alert" class="mt-1 text-xs text-danger">{editing.error}</p>{/if}
              {:else}
                <span class="block truncate" title={r.name}>{r.name}</span>
              {/if}
            </td>
            <td>
              <div class="flex items-center gap-3">
                <div class="-ml-4"><Switch label="" aria-label="Restricted: {r.name}" checked={r.restricted} onchange={(e) => restrict(r, e)} /></div>
                {#if r.restricted}
                  <Button size="sm" aria-label="Members of {r.name}" aria-haspopup="dialog" onclick={() => openMembers(r)}>
                    Members <span class="tabular-nums text-fg-2">{r.members.length}</span>
                  </Button>
                {/if}
              </div>
            </td>
            <td>
              <div class="flex items-center justify-end gap-1">
                {#if editing?.id === r.id}
                  <Button size="sm" variant="ghost" onclick={cancelRename}>Cancel</Button>
                  <Button size="sm" variant="primary" loading={busy === r.id} onclick={rename}>Save</Button>
                {:else}
                  <Button size="sm" variant="ghost" data-rename aria-label="Rename {r.name}" onclick={() => startRename(r)}>Rename</Button>
                  <IconButton icon={I.chevronUp} label="Move {r.name} up" data-move disabled={i === 0} onclick={() => move(r, -1)} />
                  <IconButton icon={I.chevronDown} label="Move {r.name} down" data-move disabled={i === rooms.length - 1} onclick={() => move(r, 1)} />
                {/if}
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if listError}<p role="alert" tabindex="-1" class="text-sm text-danger">{listError}</p>{/if}
    <div class="flex flex-col gap-1 text-sm text-fg-2">
      <p>{M.roomRestricted}</p>
      <p>{M.roomNoDelete} Alt+Up and Alt+Down move the focused row.</p>
    </div>
  </section>
</div>

<Dialog open={!!picking} title={picking ? `Who can see ${picking.name}?` : ''} onclose={() => (picking = null)}>
  {#if picking}
    {#if people.length > 10}
      <input type="search" class="control mb-3 h-8 w-full px-2.5" placeholder="Search people" aria-label="Search people" bind:value={q} />
    {/if}
    <fieldset class="max-h-80 overflow-y-auto">
      <legend class="sr-only">People who can see {picking.name}</legend>
      {#each shown as u (u.user_id)}
        <label class="flex h-9 items-center gap-3 rounded-sm px-2 hover:bg-hover">
          <input type="checkbox" value={u.user_id} bind:group={picked} />
          <span class="min-w-0 truncate text-fg">{u.display_name}</span>
          <span class="min-w-0 truncate text-xs text-fg-2">@{u.username}{u.role !== 'member' ? `, ${u.role}, sees every room` : ''}</span>
        </label>
      {:else}
        <p class="px-2 py-2 text-fg-2">{M.roomMembersNone}</p>
      {/each}
    </fieldset>
    {#if pickError}<p role="alert" class="mt-3 text-danger">{pickError}</p>{/if}
  {/if}
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (picking = null)}>Cancel</Button>
    <Button variant="primary" loading={saving} onclick={saveMembers}>Save</Button>
  {/snippet}
</Dialog>

<style>
  .sections { display: flex; flex-direction: column; gap: 32px; }
  section { display: flex; flex-direction: column; gap: 16px; }
  section + section { padding-top: 24px; border-top: 1px solid var(--line-subtle); }
  .tbl { width: 100%; table-layout: fixed; border-collapse: collapse; font-size: 13px; line-height: 20px; }
  .tbl th { height: 32px; padding: 0 8px; border-bottom: 1px solid var(--line); color: var(--fg-2); font-weight: 500; text-align: left; white-space: nowrap; }
  .tbl td { height: 48px; padding: 4px 8px; border-bottom: 1px solid var(--line-subtle); color: var(--fg); }
  .tbl :is(th, td):first-child { padding-left: 12px; }
</style>
