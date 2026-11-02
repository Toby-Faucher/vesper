import type { ClientMsg } from './gen/ClientMsg';
import type { ServerMsg } from './gen/ServerMsg';
import type { ErrorCode } from './gen/ErrorCode';

/** Plain text for the server's socket error codes. */
export const SIGNAL_MESSAGES: Record<ErrorCode, string> = {
  room_full: 'This room is full.',
  forbidden: 'You cannot join this room.',
  bad_message: 'The server did not accept that. Reload the page and try again.',
  rate_limited: 'Too many tries. Wait a moment and try again.',
};

export class Signaling {
  private ws: WebSocket;
  private queue: string[] = [];
  private msgCbs: ((m: ServerMsg) => void)[] = [];
  private closeCbs: ((byClient: boolean, code: number) => void)[] = [];
  private byClient = false;

  constructor(url: string, ws: typeof WebSocket = WebSocket) {
    this.ws = new ws(url);
    this.ws.onopen = () => this.queue.splice(0).forEach((s) => this.ws.send(s));
    this.ws.onmessage = (e) => {
      let m: ServerMsg;
      try {
        m = JSON.parse(e.data);
      } catch {
        return;
      }
      this.msgCbs.forEach((cb) => cb(m));
    };
    this.ws.onclose = (e) => this.closeCbs.forEach((cb) => cb(this.byClient, e?.code ?? 1006));
  }

  send(m: ClientMsg): void {
    const s = JSON.stringify(m);
    if (this.ws.readyState === 1) this.ws.send(s);
    else this.queue.push(s);
  }
  /** Returns a function that removes the callback. */
  onMessage(cb: (m: ServerMsg) => void): () => void {
    this.msgCbs.push(cb);
    return () => (this.msgCbs = this.msgCbs.filter((x) => x !== cb));
  }
  close(): void {
    this.byClient = true;
    this.ws.close();
  }
  /** Ends the call. The socket stays open. */
  leave(): void {
    this.send({ type: 'leave' });
  }
  /**
   * `byClient` is true when close() caused it, false for a server or network close.
   * `code` is the close code (4001 session ended, 4003 moderator disconnect). Returns an unsubscribe function.
   */
  onClose(cb: (byClient: boolean, code: number) => void): () => void {
    this.closeCbs.push(cb);
    return () => (this.closeCbs = this.closeCbs.filter((x) => x !== cb));
  }
}
