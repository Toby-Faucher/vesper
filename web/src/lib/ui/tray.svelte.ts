// The files waiting above the composer. Each file is uploaded on its own request, at most LIMIT at a time, the rest wait
// in order. Cancel aborts the request (the server deletes the partial file), Retry starts a failed one again, and the
// next Send takes the finished ones. File names and types come from the user: shown as text only, never used in a URL.
// One tray for the page (`tray`), each file with its room: a room change keeps the uploads going and the finished files
// wait in their room. Sign out and a lost session clear it (session.svelte.ts). A reload loses it: memory only.
import { ApiFailure, errorText } from '../api';
import { MAX_UPLOAD, upload } from '../chat.svelte';
import type { Attachment } from '../gen/Attachment';
import { UI_MESSAGES } from './messages-first';

export const LIMIT = 3; // uploads in flight at once; the server takes 10, then 1 every 30 s
export type TrayItem = {
  key: number;
  room: number;
  file: File;
  state: 'queued' | 'uploading' | 'done' | 'failed';
  progress: number; // 0 to 1
  error: string;
  retry: boolean; // false when trying again cannot help (too large)
  attachment: Attachment | null;
  ctrl: AbortController | null;
};

/** The plain text for a failed upload, and whether Retry can help. */
export function uploadError(e: unknown): { error: string; retry: boolean } {
  if (e instanceof ApiFailure) {
    if (e.status === 413) return { error: UI_MESSAGES.fileTooLarge, retry: false };
    if (e.status === 507) return { error: UI_MESSAGES.uploadQuota, retry: true };
    if (e.status === 429) return { error: UI_MESSAGES.uploadRate, retry: true };
    return { error: errorText(e), retry: true };
  }
  return { error: UI_MESSAGES.uploadNetwork, retry: true };
}

let keys = 0;
const pending = (i: TrayItem) => i.state === 'queued' || i.state === 'uploading';

export class Tray {
  items = $state<TrayItem[]>([]);

  of = (room: number) => this.items.filter((i) => i.room === room);
  /** An upload of this room is queued or running: Send waits. */
  busy = (room: number) => this.of(room).some(pending);
  done = (room: number) => this.of(room).filter((i) => i.state === 'done');
  /** The other rooms with an upload queued or running, in the order their files were added. */
  elsewhere = (room: number) => [...new Set(this.items.filter((i) => i.room !== room && pending(i)).map((i) => i.room))];

  private up: typeof upload;
  constructor(up: typeof upload = upload) {
    this.up = up;
  }

  add(room: number, files: File[]) {
    for (const file of files) {
      const big = file.size > MAX_UPLOAD; // the server's limit, checked here so a large file is not sent at all
      this.items.push({
        key: ++keys, room, file, state: big ? 'failed' : 'queued', progress: 0, error: big ? UI_MESSAGES.fileTooLarge : '',
        retry: !big, attachment: null, ctrl: null,
      });
    }
    this.pump();
  }

  /** Cancel (aborts a running upload) or Remove: the file leaves the tray. */
  remove(key: number) {
    this.items.find((i) => i.key === key)?.ctrl?.abort();
    this.items = this.items.filter((i) => i.key !== key);
    this.pump();
  }

  retry(key: number) {
    const i = this.items.find((x) => x.key === key);
    if (!i || i.state !== 'failed' || !i.retry) return;
    Object.assign(i, { state: 'queued', progress: 0, error: '' });
    this.pump();
  }

  /** After a send: the files it attached (by key) leave. Failed ones stay, and so does a file that finished meanwhile. */
  sent(keys: number[]) {
    this.items = this.items.filter((i) => !keys.includes(i.key));
  }

  /** Sign out: every upload of every room stops and goes. */
  clear() {
    for (const i of this.items) i.ctrl?.abort();
    this.items = [];
  }

  private pump() {
    let running = this.items.filter((i) => i.state === 'uploading').length;
    for (const i of this.items) {
      if (running >= LIMIT) break;
      if (i.state !== 'queued') continue;
      running++;
      this.start(i);
    }
  }

  private start(i: TrayItem) {
    const ctrl = new AbortController();
    Object.assign(i, { state: 'uploading', ctrl });
    this.up(i.room, i.file, (p) => (i.progress = p), ctrl.signal).then(
      (a) => Object.assign(i, { state: 'done', progress: 1, attachment: a }),
      (e) => {
        if (ctrl.signal.aborted) return; // cancelled: the item is gone already
        Object.assign(i, { state: 'failed', ...uploadError(e) });
      },
    ).finally(() => {
      i.ctrl = null;
      if (!ctrl.signal.aborted) this.pump();
    });
  }
}

export const tray = new Tray();
