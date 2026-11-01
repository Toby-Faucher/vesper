import { ApiFailure, api } from './api';
import type { ApiErrorCode } from './gen/ApiErrorCode';
import type { Attachment } from './gen/Attachment';
import type { Message } from './gen/Message';
import type { MessagePage } from './gen/MessagePage';
import type { SearchPage } from './gen/SearchPage';
import type { SearchQuery } from './gen/SearchQuery';
import type { ServerMsg } from './gen/ServerMsg';
import type { UploadDone } from './gen/UploadDone';

/** The loaded slice of one room's history, oldest first. `moreAfter` is set after a jump into the past. */
export type RoomLog = { messages: Message[]; moreBefore: boolean; moreAfter: boolean };
export type ChatState = {
  rooms: Record<number, RoomLog>;
  /** Room id, then user id, then when the "is typing" line ends (ms). */
  typing: Record<number, Record<string, number>>;
};

export const TYPING_MS = 5000;
export const MAX_UPLOAD = 25 * 1024 * 1024;
const PAGE = 50;

/**
 * True when `incoming` is older than `held` and must not replace it. Events for one message can arrive out of
 * order; the server raises `updated_at` on every change, so the larger one is the newer copy.
 */
export const stale = (held: Message, incoming: Message) => held.updated_at > incoming.updated_at;

/** Inserts or replaces `m` by id, keeping the list sorted. Returns the same list when nothing changed. */
export function upsert(list: Message[], m: Message, insert = true): Message[] {
  const i = list.findIndex((x) => x.id >= m.id);
  if (i >= 0 && list[i].id === m.id) return stale(list[i], m) ? list : list.map((x, j) => (j === i ? m : x));
  if (!insert) return list;
  return i < 0 ? [...list, m] : [...list.slice(0, i), m, ...list.slice(i)];
}

/** The newest page merged into what is held. With a gap between them the held slice is dropped. */
export function mergeLatest(log: RoomLog | undefined, page: MessagePage): RoomLog {
  const fresh = { messages: page.messages, moreBefore: page.more_before, moreAfter: false };
  const first = page.messages[0];
  if (!log || log.moreAfter || !first || !log.messages.some((m) => m.id >= first.id)) return fresh;
  const held = new Map(log.messages.map((m) => [m.id, m]));
  const older = log.messages.filter((m) => m.id < first.id);
  const merged = page.messages.map((p) => {
    const h = held.get(p.id);
    return h && stale(h, p) ? h : p;
  });
  return { messages: [...older, ...merged], moreBefore: older.length ? log.moreBefore : page.more_before, moreAfter: false };
}

export function applyChatMsg(s: ChatState, m: ServerMsg): ChatState {
  switch (m.type) {
    case 'message_new':
    case 'message_updated': {
      const msg = m.message;
      const typing = { ...s.typing, [msg.room_id]: { ...s.typing[msg.room_id] } };
      if (m.type === 'message_new') delete typing[msg.room_id][msg.author_id];
      const log = s.rooms[msg.room_id];
      // Not loaded, or showing an older slice: the message is fetched when the newest page loads.
      if (!log || (m.type === 'message_new' && log.moreAfter)) return { ...s, typing };
      const messages = upsert(log.messages, msg, m.type === 'message_new');
      return { typing, rooms: messages === log.messages ? s.rooms : { ...s.rooms, [msg.room_id]: { ...log, messages } } };
    }
    case 'typing':
      return { ...s, typing: { ...s.typing, [m.room_id]: { ...s.typing[m.room_id], [m.user_id]: Date.now() + TYPING_MS } } };
    default:
      return s; // `read` is unread state, kept in state.ts
  }
}

/** User ids typing in a room right now. */
export const typers = (s: ChatState, roomId: number, now = Date.now()) =>
  Object.entries(s.typing[roomId] ?? {}).filter(([, until]) => until > now).map(([id]) => id);

/** A search snippet split at its \u0002 ... \u0003 match markers, for rendering as text with <mark>. */
export function marks(snippet: string): { text: string; mark: boolean }[] {
  const out: { text: string; mark: boolean }[] = [];
  let mark = false;
  for (const part of snippet.split(/([\u0002\u0003])/)) {
    if (part === '\u0002') mark = true;
    else if (part === '\u0003') mark = false;
    else if (part) out.push({ text: part, mark });
  }
  return out;
}

/** The scrollTop that keeps the same content in view after rows were added above it. */
export const anchorTop = (oldTop: number, oldHeight: number, newHeight: number) => oldTop + (newHeight - oldHeight);

export const chat = $state({
  state: { rooms: {}, typing: {} } as ChatState,
  /** The room shown, refetched after a reconnect. */
  active: null as number | null,
  /** Unsent text by room id, in memory only. */
  drafts: {} as Record<number, string>,
});

const setLog = (roomId: number, log: RoomLog) => (chat.state = { ...chat.state, rooms: { ...chat.state.rooms, [roomId]: log } });
const history = (roomId: number, q = '') => api<MessagePage>('GET', `/api/rooms/${roomId}/messages?limit=${PAGE}${q}`);

/** Loads the newest page of a room, merged into what is held. */
export async function loadLatest(roomId: number): Promise<void> {
  const page = await history(roomId);
  setLog(roomId, mergeLatest(chat.state.rooms[roomId], page));
}

/** After a reconnect events may have been missed: drop other rooms and refetch the shown one. */
export function resync(): Promise<void> {
  const keep = chat.active;
  const kept = keep === null ? undefined : chat.state.rooms[keep];
  chat.state = { rooms: kept ? { [keep!]: kept } : {}, typing: {} };
  return keep === null ? Promise.resolve() : loadLatest(keep).catch(() => {});
}

export async function loadOlder(roomId: number): Promise<void> {
  const log = chat.state.rooms[roomId];
  if (!log?.moreBefore || !log.messages.length) return;
  const page = await history(roomId, `&before=${log.messages[0].id}`);
  const now = chat.state.rooms[roomId] ?? log;
  const first = now.messages[0]?.id ?? Infinity;
  setLog(roomId, { ...now, messages: [...page.messages.filter((m) => m.id < first), ...now.messages], moreBefore: page.more_before });
}

export async function loadNewer(roomId: number): Promise<void> {
  const log = chat.state.rooms[roomId];
  if (!log?.moreAfter) return;
  const page = await history(roomId, `&after=${log.messages.at(-1)?.id ?? 0}`);
  const now = chat.state.rooms[roomId] ?? log;
  let messages = now.messages;
  for (const m of page.messages) messages = upsert(messages, m);
  setLog(roomId, { ...now, messages, moreAfter: page.more_after });
}

/** Shows the history around one message (a search hit). */
export async function jumpTo(roomId: number, id: number): Promise<void> {
  const page = await history(roomId, `&around=${id}`);
  setLog(roomId, { messages: page.messages, moreBefore: page.more_before, moreAfter: page.more_after });
}

function apply(m: Message): void {
  chat.state = applyChatMsg(chat.state, { type: 'message_updated', message: m });
}

export async function send(roomId: number, body: string, replyTo: number | null = null, attachments: string[] = []): Promise<void> {
  const m = await api<Message>('POST', `/api/rooms/${roomId}/messages`, { body, reply_to: replyTo, attachments });
  chat.state = applyChatMsg(chat.state, { type: 'message_new', message: m });
}
export const edit = async (id: number, body: string) => apply(await api<Message>('PATCH', `/api/messages/${id}`, { body }));
export const remove = (id: number) => api<void>('DELETE', `/api/messages/${id}`); // the message_updated event shows it
export const react = async (id: number, emoji: string, on: boolean) =>
  apply(await api<Message>('PUT', `/api/messages/${id}/reaction`, { emoji, on }));
export const pin = async (id: number, on: boolean) => apply(await api<Message>('PUT', `/api/messages/${id}/pin`, { on }));
export const markRead = (roomId: number, lastReadId: number) =>
  api<void>('PUT', `/api/rooms/${roomId}/read`, { last_read_id: lastReadId });

/** Uploads one file as a raw body. XHR, because fetch has no upload progress. Rejects with ApiFailure. */
export function upload(roomId: number, file: File, onProgress?: (done: number) => void): Promise<Attachment> {
  if (file.size > MAX_UPLOAD) return Promise.reject(new ApiFailure(413, 'too_large'));
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest();
    x.open('POST', `/api/rooms/${roomId}/uploads?name=${encodeURIComponent(file.name || 'file')}`);
    x.upload.onprogress = (e) => e.lengthComputable && onProgress?.(e.loaded / e.total);
    x.onload = () => {
      let body: unknown;
      try {
        body = JSON.parse(x.responseText);
      } catch {
        body = null;
      }
      if (x.status >= 200 && x.status < 300 && body) resolve((body as UploadDone).attachment);
      else {
        const code = (body as { code?: unknown } | null)?.code;
        reject(new ApiFailure(x.status, typeof code === 'string' ? (code as ApiErrorCode) : 'internal'));
      }
    };
    x.onerror = () => reject(new TypeError('network'));
    x.send(file);
  });
}

export function search(q: SearchQuery): Promise<SearchPage> {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(q)) if (v !== null && v !== '') p.set(k, String(v));
  return api<SearchPage>('GET', `/api/search?${p}`);
}
