import { ApiFailure, api } from './api';
import { applyChatMsg, chat, resync } from './chat.svelte';
import { loginHref } from './route';
import { onServerMsg } from './ui/notify';
import { recentKey } from './ui/recent';
import { tray } from './ui/tray.svelte';
import { Signaling } from './signaling';
import { applyServerMsg, emptyState, type AppState } from './state';
import type { Me } from './gen/Me';

/** The current path, without the query and the fragment. App.svelte renders from it. */
export const route = $state({ path: location.pathname });

/** Goes to `to` (a path, maybe with a query and a fragment). `replace` swaps the current history entry, for redirects, so
 * Back does not loop. An entry pushed here is marked `app`, so a page can tell that Back stays in the app (`cameInApp`). */
export function navigate(to: string, replace = false): void {
  if (location.pathname + location.search + location.hash === to) return void (route.path = to.split(/[?#]/)[0]);
  // A move inside Settings or Admin replaces, so one visit is one entry and Close (Back) leaves the page.
  const page = (p: string) => /^\/(settings|admin)(\/|$)/.exec(p)?.[1];
  if (replace || (page(location.pathname) && page(location.pathname) === page(to))) history.replaceState(history.state, '', to);
  else history.pushState({ app: true }, '', to);
  route.path = to.split(/[?#]/)[0];
}

/** True when this history entry came from a click in the app, so the entry before it is an app page too. */
export const cameInApp = () => !!(history.state as { app?: boolean } | null)?.app;

/** Settings and Admin close back to where they were opened from: Back when that was an app page (so Back after Close does
 * not show the same room twice), else the last room (`/` opens it) in place of this entry. */
export function closePage(): void {
  if (cameInApp()) history.back();
  else navigate('/', true);
}

/** The localStorage key of the room `/` opens, per account, so a second account on this browser does not get it. */
export const lastRoomKey = (userId: string) => `vesper.lastRoom.${userId}`;
const storage = (() => { try { return localStorage; } catch { return null; } })();

/** Reads the invite or reset code from the URL fragment and removes it from the address bar and history. */
export function takeInvite(): string {
  const code = location.hash.slice(1);
  if (location.hash) history.replaceState(null, '', location.pathname + location.search);
  return code;
}

export const wsUrl = () => `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`;

// Retry n waits a random 50 to 100% of min(30 s, 2 s * 2^n), so tabs do not retry in step after a restart.
export const backoffMs = (n: number) => Math.min(30000, 2000 * 2 ** n) * (0.5 + Math.random() / 2);
// The retry count resets only after a socket stayed up this long, so accept-then-drop does not loop at 1 s.
const STABLE_MS = 10000;
let timer: ReturnType<typeof setTimeout> | undefined;
let tries = 0;
let gen = 0; // stop() bumps it, so a retry already asking /api/me does not open a socket afterwards

export const session = $state({
  state: emptyState() as AppState,
  sig: null as Signaling | null,
  /** True between the server's hello and the socket closing. */
  online: false,
  /** Why the socket is not coming back, shown to the user. */
  notice: '',
  /** When the next reconnect try starts (ms since epoch), while one waits. */
  retryAt: null as number | null,
  /** The server now runs another build than the one this page started with: the socket is closed for good and the page
   * asks for a reload. Never reloads by itself, so a mismatch can not loop. */
  stale: false,

  /** Opens the one socket for this page. Reconnects after a drop, except on 4001 (go to sign in) and 4003. */
  connect(): void {
    if (session.sig || timer || session.stale) return;
    session.notice = '';
    tries = 0;
    open();
  },

  /** Skips the wait before the next reconnect try. */
  retryNow(): void {
    if (session.retryAt === null) return;
    clearTimeout(timer);
    attempt();
  },

  async logout(): Promise<void> {
    const id = session.state.me?.user.user_id;
    if (id) try { storage?.removeItem(lastRoomKey(id)); storage?.removeItem(recentKey(id)); } catch { /* storage blocked */ }
    stop();
    await api('POST', '/api/logout').catch(() => {}); // signed out either way
    // The server drops this device's push subscription with the session; unregistering also ends it in the browser.
    await navigator.serviceWorker?.getRegistration().then((r) => r?.unregister()).catch(() => {});
    toLogin('');
  },
});

function open(): void {
  const sig = new Signaling(wsUrl());
  session.sig = sig;
  let upAt = 0;
  sig.onMessage((m) => {
    if (m.type === 'hello' && !sameBuild(m.me, true)) return; // its messages may be of a kind this code does not know
    if (m.type === 'hello') {
      upAt = Date.now();
      session.online = true;
    }
    const prev = session.state;
    session.state = applyServerMsg(prev, m);
    chat.state = applyChatMsg(chat.state, m);
    onServerMsg(m, prev); // mention notifications and the join and leave sounds
    if (m.type === 'hello') void resync(); // events may have been missed while the socket was down
  });
  sig.onClose((byClient, code) => {
    if (session.sig !== sig) return;
    if (upAt && Date.now() - upAt >= STABLE_MS) tries = 0;
    session.sig = null;
    session.online = false;
    if (byClient) return;
    if (code === 4001) toLogin('Your session ended. Sign in again.');
    else if (code === 4003) session.notice = 'A moderator disconnected you.';
    else retry();
  });
}

// A refused upgrade (signed out) looks like a network drop, so ask /api/me before each new socket.
function retry(): void {
  const ms = backoffMs(tries++);
  session.retryAt = Date.now() + ms;
  timer = setTimeout(attempt, ms);
}

// `timer` stays set until the answer, so connect() does not open a second socket meanwhile.
function attempt(): void {
  const g = gen;
  session.retryAt = null;
  api<Me>('GET', '/api/me').then(
    (me) => {
      if (g !== gen) return;
      timer = undefined;
      if (sameBuild(me, false)) open();
    },
    (e) => {
      if (g !== gen) return;
      timer = undefined;
      if (e instanceof ApiFailure && e.status === 401) toLogin('Your session ended. Sign in again.');
      else retry();
    },
  );
}

let build: string | null = null; // the server build of this page load's first hello

/** False, and the socket stops, once the server is on another build than this page started with. Build ids, not versions,
 * so a rebuild of one version counts too. An older server sends none: that is a change as well. Only a hello sets the
 * baseline: an /api/me before the first hello (the first socket closed early) is compared with nothing and passes. */
function sameBuild(me: Me | undefined, hello: boolean): boolean {
  const b = me?.server?.build ?? '';
  if (build === null && !hello) return true;
  build ??= b;
  if (b === build) return true;
  stop();
  session.stale = true;
  return false;
}

function stop(): void {
  gen++;
  clearTimeout(timer);
  timer = undefined;
  session.retryAt = null;
  const sig = session.sig;
  session.sig = null;
  session.online = false;
  sig?.close();
}

function toLogin(notice: string): void {
  stop();
  session.state = emptyState();
  Object.assign(chat, { state: { rooms: {}, typing: {} }, active: null, drafts: {} }); // the next account sees none of it
  tray.clear(); // and the uploads stop
  session.notice = notice;
  navigate(notice ? loginHref(location.pathname) : '/login', true); // a sign in after a lost session returns to this page
}
