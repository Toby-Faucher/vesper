import { ApiFailure, api } from './api';
import { Signaling } from './signaling';
import { applyServerMsg, emptyState, type AppState } from './state';

/** The current path. App.svelte renders from it. */
export const route = $state({ path: location.pathname });

export function navigate(path: string): void {
  if (location.pathname !== path) history.pushState(null, '', path);
  route.path = path;
}

export const wsUrl = () => `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`;

const BACKOFF_MS = [1000, 2000, 5000, 10000];
let timer: ReturnType<typeof setTimeout> | undefined;
let tries = 0;

export const session = $state({
  state: emptyState() as AppState,
  sig: null as Signaling | null,
  /** True between the server's hello and the socket closing. */
  online: false,
  /** Why the socket is not coming back, shown to the user. */
  notice: '',

  /** Opens the one socket for this page. Reconnects after a drop, except on 4001 (go to sign in) and 4003. */
  connect(): void {
    if (session.sig || timer) return;
    session.notice = '';
    tries = 0;
    open();
  },

  async logout(): Promise<void> {
    stop();
    await api('POST', '/api/logout').catch(() => {}); // signed out either way
    toLogin('');
  },
});

function open(): void {
  const sig = new Signaling(wsUrl());
  session.sig = sig;
  sig.onMessage((m) => {
    if (m.type === 'hello') {
      tries = 0;
      session.online = true;
    }
    session.state = applyServerMsg(session.state, m);
  });
  sig.onClose((byClient, code) => {
    if (session.sig !== sig) return;
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
  timer = setTimeout(() => {
    api('GET', '/api/me').then(
      () => {
        timer = undefined;
        open();
      },
      (e) => {
        timer = undefined;
        if (e instanceof ApiFailure && e.status === 401) toLogin('Your session ended. Sign in again.');
        else retry();
      },
    );
  }, BACKOFF_MS[Math.min(tries++, BACKOFF_MS.length - 1)]);
}

function stop(): void {
  clearTimeout(timer);
  timer = undefined;
  const sig = session.sig;
  session.sig = null;
  session.online = false;
  sig?.close();
}

function toLogin(notice: string): void {
  stop();
  session.state = emptyState();
  session.notice = notice;
  navigate('/login');
}
