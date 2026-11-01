import type { Message } from '../gen/Message';
import type { ServerMsg } from '../gen/ServerMsg';
import type { AppState } from '../state';
import { href } from '../route';
import { chat, jumpTo } from '../chat.svelte';
import { navigate, session } from '../session.svelte';
import { voice } from '../voice.svelte';
import { chime } from './chime';

// Mention notifications and the join and leave sounds. The browser is asked for permission only from a click
// (requestMentions); nothing here asks on load.

export const NOTIFY_KEY = 'vesper.notify';
export type NotifyPrefs = { mentions: boolean; sounds: boolean };
const DEFAULTS: NotifyPrefs = { mentions: false, sounds: false };
const storage = (() => { try { return localStorage; } catch { return null; } })();

/** Read on every use, so a change in another tab applies here too. */
export function loadPrefs(): NotifyPrefs {
  try {
    const p = JSON.parse(storage?.getItem(NOTIFY_KEY) ?? '{}');
    return { mentions: p.mentions === true, sounds: p.sounds === true };
  } catch {
    return { ...DEFAULTS };
  }
}
export function savePrefs(p: NotifyPrefs): void {
  try { storage?.setItem(NOTIFY_KEY, JSON.stringify(p)); } catch { /* storage blocked: the setting lasts this page */ }
}

export const supported = () => typeof Notification !== 'undefined';
export const permission = (): NotificationPermission | 'unsupported' => (supported() ? Notification.permission : 'unsupported');

/** A mention of me by someone else, not deleted, mentions on and allowed, and I am not looking at that room. */
export function shouldNotify(
  m: Message,
  ctx: { myId: string; activeRoom: number | null; hidden: boolean; prefs: NotifyPrefs; permission: NotificationPermission },
): boolean {
  return (
    m.mentions.includes(ctx.myId) && m.author_id !== ctx.myId && !m.deleted && ctx.prefs.mentions &&
    ctx.permission === 'granted' && (ctx.hidden || ctx.activeRoom !== m.room_id)
  );
}

/** Within this long after a hello the joins and leaves are a reconnect catching up, not people moving. */
export const QUIET_AFTER_HELLO_MS = 3000;

/** `leftRoom` is the room the peer was in before a peer_left (that message carries no room). */
export function shouldChime(
  m: ServerMsg,
  ctx: { joinedRoom: number | null; myId: string; online: boolean; sinceHelloMs: number; prefs: NotifyPrefs; leftRoom?: number | null },
): 'join' | 'leave' | null {
  if (!ctx.prefs.sounds || !ctx.online || ctx.joinedRoom === null || ctx.sinceHelloMs < QUIET_AFTER_HELLO_MS) return null;
  if (m.type === 'peer_joined') return m.peer.user_id !== ctx.myId && m.peer.room_id === ctx.joinedRoom ? 'join' : null;
  if (m.type === 'peer_left') return m.user_id !== ctx.myId && ctx.leftRoom === ctx.joinedRoom ? 'leave' : null;
  return null;
}

/** Asks the browser, then turns mentions on when allowed. Only ever call it from a click handler. */
export async function requestMentions(): Promise<NotificationPermission> {
  if (!supported()) return 'denied';
  const p = await Notification.requestPermission();
  if (p === 'granted') savePrefs({ ...loadPrefs(), mentions: true });
  return p;
}

export const titleFor = (mentions: number) => (mentions > 0 ? `(${mentions}) Vesper` : 'Vesper');

/** The summed mention counts of every room. */
export const mentionCount = (s: AppState) => Object.values(s.unread).reduce((n, u) => n + u.mentions, 0);

let helloAt = 0;

/** Called by the session after each server message is applied; `prev` is the state before it. */
export function onServerMsg(m: ServerMsg, prev: AppState): void {
  if (m.type === 'hello') helloAt = Date.now();
  const myId = session.state.me?.user.user_id ?? '';
  if (m.type === 'message_new') {
    const ctx = { myId, activeRoom: chat.active, hidden: document.hidden || !document.hasFocus(), prefs: loadPrefs() };
    if (supported() && shouldNotify(m.message, { ...ctx, permission: Notification.permission })) void show(m.message).catch(() => {});
    return;
  }
  const leftRoom = m.type === 'peer_left' ? prev.voice.find((p) => p.user_id === m.user_id)?.room_id ?? null : null;
  const kind = shouldChime(m, {
    joinedRoom: voice.joinedRoom, myId, online: session.online, sinceHelloMs: Date.now() - helloAt, prefs: loadPrefs(), leftRoom,
  });
  if (kind) chime(kind, voice.settings.speakerId);
}

async function show(m: Message): Promise<void> {
  // Loaded on the first notification, so the Markdown parser stays out of the first load.
  const [{ parse }, { plain }] = await Promise.all([import('../markdown'), import('./timeline')]);
  const s = session.state;
  const name = s.users[m.author_id]?.display_name ?? 'Someone';
  const room = s.rooms.find((r) => r.id === m.room_id)?.name ?? 'a room';
  try {
    const n = new Notification(`${name} mentioned you in ${room}`, {
      body: plain(parse(m.body)).trim().slice(0, 120),
      tag: String(m.room_id),
      icon: '/icon-192.png',
    });
    n.onclick = () => {
      window.focus();
      n.close();
      chat.jump = m.id; // the timeline scrolls to it and flashes it once the slice lands
      jumpTo(m.room_id, m.id).catch(() => (chat.jump = null));
      navigate(href({ name: 'room', id: m.room_id }));
    };
  } catch { /* some browsers allow notifications only from a service worker: no notification then */ }
}

/** The test button in Settings. */
export function testNotification(): void {
  try { new Notification('Vesper', { body: 'Mentions show like this.', icon: '/icon-192.png' }); } catch { /* see show() */ }
}
