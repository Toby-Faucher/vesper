<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { ApiFailure, api, errorText } from '../../lib/api';
  import { session } from '../../lib/session.svelte';
  import { MAX_BIO, MAX_PICTURE, PICTURE_TYPES, SLOT_HEX, avatarSlot, profilePatch, squarePicture } from '../../lib/ui/avatar';
  import { UI_MESSAGES as M, relTime } from '../../lib/ui/messages';
  import { type PwError, pwCheck, pwFailure } from './account';
  import Avatar from '../../lib/ui/c/Avatar.svelte';
  import Button from '../../lib/ui/c/Button.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import PasswordInput from '../../lib/ui/c/PasswordInput.svelte';
  import TextInput from '../../lib/ui/c/TextInput.svelte';
  import Swatches from '../Swatches.svelte';
  import type { Device } from '../../lib/gen/Device';
  import type { UserInfo } from '../../lib/gen/UserInfo';
  // Settings > Account: picture, name, colour and bio, the password, and the devices signed in to this account.
  const me = $derived(session.state.me!.user);

  let name = $state(session.state.me!.user.display_name);
  let slot = $state(avatarSlot(session.state.me!.user));
  let bio = $state(session.state.me!.user.bio);
  let saving = $state(false);
  let profileError = $state('');
  let saved = $state(false);
  async function saveProfile(e: Event) {
    e.preventDefault();
    if (saving) return;
    const patch = profilePatch(me, name, slot, bio);
    profileError = '';
    if (Object.keys(patch).length) {
      saving = true;
      try {
        const user = await api<UserInfo>('PATCH', '/api/me', patch);
        setMe(user);
      } catch (err) {
        profileError = errorText(err);
      } finally {
        saving = false;
      }
    }
    saved = !profileError;
  }

  // The socket's user_updated does the same a moment later; this shows it at once.
  const setMe = (user: UserInfo) => {
    session.state.me = { ...session.state.me!, user };
    session.state.users = { ...session.state.users, [user.user_id]: user };
  };

  // The picture is saved as soon as it is picked or dropped: cropped to the centre square and redrawn small first.
  let picBusy = $state<'' | 'upload' | 'remove'>('');
  let picError = $state('');
  let picDone = $state('');
  let dragging = $state(false);
  let picInput: HTMLInputElement;
  async function setPicture(file: File | undefined) {
    if (!file || picBusy) return;
    [picError, picDone, picBusy] = ['', '', 'upload'];
    try {
      const pic = await squarePicture(file);
      if (pic.size > MAX_PICTURE) throw new Error('large');
      setMe(await api<UserInfo>('PUT', '/api/me/avatar', pic));
      picDone = M.pictureSaved;
    } catch (err) {
      const why = err instanceof Error ? err.message : '';
      picError =
        why === 'type' ? M.pictureType
        : why === 'unreadable' ? M.pictureUnreadable
        : why === 'large' || (err instanceof ApiFailure && err.code === 'too_large') ? M.pictureTooLarge
        : err instanceof ApiFailure && err.code === 'bad_request' ? M.pictureRefused
        : errorText(err);
    } finally {
      picBusy = '';
      picInput.value = ''; // picking the same file again still fires change
    }
  }
  async function removePicture() {
    if (picBusy) return;
    [picError, picDone, picBusy] = ['', '', 'remove'];
    try {
      setMe(await api<UserInfo>('DELETE', '/api/me/avatar'));
      picDone = M.pictureRemoved;
    } catch (err) {
      picError = errorText(err);
    } finally {
      picBusy = '';
    }
  }
  const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files');
  function onDrop(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.stopPropagation();
    dragging = false;
    void setPicture(e.dataTransfer!.files[0]);
  }

  let oldPw = $state('');
  let newPw = $state('');
  let confirmPw = $state('');
  let changing = $state(false);
  let pwError = $state<PwError | null>(null);
  let pwDone = $state(false);
  let pwForm: HTMLFormElement;
  async function changePassword(e: Event) {
    e.preventDefault();
    if (changing) return;
    pwDone = false;
    pwError = pwCheck(newPw, confirmPw);
    if (!pwError) {
      changing = true;
      try {
        await api('POST', '/api/me/password', { old: oldPw, new: newPw });
        pwDone = true;
        void loadDevices(); // the others are gone now
      } catch (err) {
        pwError = pwFailure(err);
      } finally {
        [oldPw, newPw, confirmPw, changing] = ['', '', '', false]; // done or failed, no password stays in the form
      }
    }
    if (pwError) {
      await tick();
      pwForm.querySelector<HTMLElement>('[role=alert]')?.focus();
    }
  }
  const pwErr = (at: 'old' | 'new' | 'confirm') => (pwError?.at === at ? pwError.text : undefined);

  let devices = $state<Device[] | null>(null);
  let devicesFailed = $state(false);
  let revokeError = $state('');
  let now = $state(Date.now());
  async function loadDevices() {
    try {
      const list = await api<Device[]>('GET', '/api/sessions');
      devices = list.sort((a, b) => +b.current - +a.current); // this device first, then by last seen (server order)
      devicesFailed = false;
      now = Date.now();
    } catch {
      devicesFailed = true;
    }
  }
  let revoking = $state<number | null>(null);
  async function signOut(d: Device) {
    if (d.current) return session.logout(); // like Log out: back to the sign-in page
    if (revoking !== null) return; // one at a time: a second click would DELETE a row that is already gone
    revokeError = '';
    revoking = d.id;
    try {
      await api('DELETE', `/api/sessions/${d.id}`);
      devices = devices!.filter((x) => x.id !== d.id);
    } catch {
      revokeError = M.signOutFailed;
    } finally {
      revoking = null;
    }
  }
  const dateOf = (ms: number) => new Date(ms).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
  onMount(() => void loadDevices());
</script>

<div class="sections">
  <section role="group" aria-labelledby="account-profile">
    <h3 id="account-profile" class="text-base font-medium text-fg">Profile</h3>
    <div
      class="pic flex max-w-80 items-center gap-3 rounded-md p-3"
      class:dragging
      role="group"
      aria-label="Profile picture"
      ondragover={(e) => { if (hasFiles(e)) { e.preventDefault(); dragging = true; } }}
      ondragleave={() => (dragging = false)}
      ondrop={onDrop}
    >
      <Avatar user={{ ...me, display_name: name || me.username, accent: SLOT_HEX[slot - 1] }} size={40} />
      <div class="min-w-0 flex-1">
        <p class="truncate text-base font-medium text-fg">{name || me.username}</p>
        <p class="truncate text-xs text-fg-2">Signed in as {me.username}</p>
      </div>
    </div>
    <div class="flex max-w-80 flex-col gap-2">
      <input bind:this={picInput} type="file" accept={PICTURE_TYPES.join(',')} hidden onchange={(e) => setPicture(e.currentTarget.files?.[0])} />
      <div class="flex flex-wrap items-center gap-2">
        <Button size="sm" loading={picBusy === 'upload'} onclick={() => picInput.click()}>{me.avatar ? 'Change picture' : 'Upload picture'}</Button>
        {#if me.avatar}<Button size="sm" variant="ghost" loading={picBusy === 'remove'} onclick={removePicture}>Remove picture</Button>{/if}
        <p role="status" class="text-sm text-fg-2">{picDone}</p>
      </div>
      <p class="text-xs text-fg-2">PNG, JPEG, WebP or GIF. You can also drop one on the card above. It is cropped to a square.</p>
      {#if picError}<p role="alert" class="text-sm text-danger">{picError}</p>{/if}
    </div>
    <form onsubmit={saveProfile} class="flex max-w-80 flex-col gap-5">
      <Field label="Display name" help="1 to 32 characters." error={profileError || undefined}>
        <TextInput autocomplete="nickname" bind:value={name} oninput={() => (saved = false)} required maxlength={32} />
      </Field>
      <div onchange={() => (saved = false)}><Swatches bind:slot gap="var(--bg-app)" /></div>
      <Field label="Bio" value="{[...bio].length} / {MAX_BIO}" help="Up to {MAX_BIO} characters, shown on your person card.">
        <TextInput bind:value={bio} oninput={() => (saved = false)} maxlength={MAX_BIO} />
      </Field>
      <div class="flex items-center gap-3">
        <Button type="submit" loading={saving}>Save</Button>
        <p role="status" class="text-sm text-fg-2">{saved ? M.profileSaved : ''}</p>
      </div>
    </form>
  </section>

  <section role="group" aria-labelledby="account-password">
    <h3 id="account-password" class="text-base font-medium text-fg">Password</h3>
    <form onsubmit={changePassword} bind:this={pwForm} class="flex flex-col gap-4">
      <input type="text" autocomplete="username" value={me.username} hidden readonly />
      <div class="flex max-w-80 flex-col gap-4">
        <Field label="Current password" error={pwErr('old')}>
          <PasswordInput autocomplete="current-password" bind:value={oldPw} required />
        </Field>
        <Field label="New password" help="8 to 128 characters." error={pwErr('new')}>
          <PasswordInput autocomplete="new-password" bind:value={newPw} required />
        </Field>
        <Field label="Confirm new password" error={pwErr('confirm')}>
          <PasswordInput autocomplete="new-password" bind:value={confirmPw} required />
        </Field>
      </div>
      <div class="flex items-center gap-3">
        <Button type="submit" loading={changing}>Change password</Button>
        <p role="status" class="text-sm text-fg-2">{pwDone ? M.passwordChanged : ''}</p>
      </div>
      {#if pwError?.at === 'form'}<p role="alert" tabindex="-1" class="-mt-2 text-sm text-danger">{pwError.text}</p>{/if}
    </form>
  </section>

  <section role="group" aria-labelledby="account-devices">
    <div>
      <h3 id="account-devices" class="text-base font-medium text-fg">Devices</h3>
      <p class="mt-1 text-sm text-fg-2">Browsers signed in to this account. Signing one out ends its session there.</p>
    </div>
    {#if devicesFailed}
      <p role="alert" class="text-sm text-fg-2">{M.devicesFailed}</p>
    {:else if !devices}
      <p class="text-sm text-fg-2">{M.devicesLoading}</p>
    {:else if !devices.length}
      <p class="text-sm text-fg-2">{M.devicesNone}</p>
    {:else}
      <ul aria-label="Signed-in devices" class="devices">
        {#each devices as d (d.id)}
          <li class="flex items-center gap-4 px-3 py-2.5">
            <div class="min-w-0 flex-1" id="device-{d.id}">
              <p class="flex items-center gap-2 text-sm font-medium text-fg">
                <span class="truncate">{d.label}</span>
                {#if d.current}<span class="shrink-0 rounded-sm bg-selected px-1.5 text-xs text-fg-2">This device</span>{/if}
              </p>
              <p class="text-xs text-fg-2">
                Last seen <span title={new Date(d.last_seen_at).toLocaleString()}>{relTime(d.last_seen_at, now)}</span>. Signed in
                {dateOf(d.created_at)}.
              </p>
            </div>
            <Button size="sm" aria-describedby="device-{d.id}" loading={revoking === d.id} onclick={() => signOut(d)}>Sign out</Button>
          </li>
        {/each}
      </ul>
      {#if revokeError}<p role="alert" class="text-sm text-danger">{revokeError}</p>{/if}
    {/if}
  </section>

  <section role="group" aria-labelledby="account-welcome">
    <div>
      <h3 id="account-welcome" class="text-base font-medium text-fg">Welcome steps</h3>
      <p class="mt-1 text-sm text-fg-2">Your name and colour, a microphone check and mention notifications, as after signing up.</p>
    </div>
    <p><a class="text-accent-fg underline underline-offset-2" href="/welcome">Run the welcome steps again</a></p>
  </section>
</div>

<style>
  .sections { display: flex; flex-direction: column; gap: 32px; }
  .pic { background: var(--bg-raised); outline: 2px dashed transparent; outline-offset: -2px; }
  .pic.dragging { outline-color: var(--accent); }
  section { display: flex; flex-direction: column; gap: 16px; }
  section + section { padding-top: 24px; border-top: 1px solid var(--line-subtle); }
  .devices { border: 1px solid var(--line); border-radius: var(--radius); }
  .devices > li + li { border-top: 1px solid var(--line-subtle); }
</style>
