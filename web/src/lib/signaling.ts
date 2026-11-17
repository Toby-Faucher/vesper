import type { ClientMsg } from './gen/ClientMsg';
import type { ServerMsg } from './gen/ServerMsg';
import type { ErrorCode } from './gen/ErrorCode';

/** Plain text for the server's socket error codes. */
export const SIGNAL_MESSAGES: Record<ErrorCode, string> = {
  room_full: 'This room is full.',
  forbidden: 'You cannot join this room.',
  bad_message: 'The server did not accept that. Reload the page and try again.',
  rate_limited: 'Too many tries. Wait a moment and try again.',
  share_cap: 'Too many screens are being shared right now.',
  internal: 'Something went wrong on the server. Try again in a moment.',
};

/** The close code of a socket given up on for silence (`CLOSE_STALLED` on the server): the server keeps the call for a resume. */
export const STALLED = 4004;
/** A `ping` goes out this often while the socket is open, so a working socket is never silent for long. */
export const PING_MS = 10_000;
/** No message at all from the server for this long: the socket is dead even if the browser does not know it yet. */
export const SILENT_MS = 25_000;

export class Signaling {
  private ws: WebSocket;
  private queue: string[] = [];
  private msgCbs: ((m: ServerMsg) => void)[] = [];
  private closeCbs: ((byClient: boolean, code: number) => void)[] = [];
  private byClient = false;
  private pinger: ReturnType<typeof setInterval> | undefined;
  private silence: ReturnType<typeof setTimeout> | undefined;

  constructor(url: string, ws: typeof WebSocket = WebSocket) {
    this.ws = new ws(url);
    this.ws.onopen = () => {
      this.queue.splice(0).forEach((s) => this.ws.send(s));
      this.pinger = setInterval(() => this.send({ type: 'ping' }), PING_MS);
      this.heard();
    };
    this.ws.onmessage = (e) => {
      this.heard();
      let m: ServerMsg;
      try {
        m = JSON.parse(e.data);
      } catch {
        return;
      }
      this.msgCbs.forEach((cb) => cb(m));
    };
    this.ws.onclose = (e) => this.ended(this.byClient, e?.code ?? 1006);
  }

  private heard(): void {
    clearTimeout(this.silence);
    this.silence = setTimeout(() => this.stalled(), SILENT_MS);
  }
  // A browser keeps a silent socket open for minutes, and close() on it only fires onclose after the close handshake,
  // which never comes: so this socket counts as closed now, and the close goes out for whoever still listens.
  private stalled(): void {
    this.ws.onclose = this.ws.onmessage = null;
    this.ws.close(STALLED);
    this.ended(false, STALLED);
  }
  private ended(byClient: boolean, code: number): void {
    clearInterval(this.pinger);
    clearTimeout(this.silence);
    this.closeCbs.forEach((cb) => cb(byClient, code));
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
    clearInterval(this.pinger);
    clearTimeout(this.silence);
    this.ws.close();
  }
  /** Ends the call. The socket stays open. */
  leave(): void {
    this.send({ type: 'leave' });
  }
  /**
   * `byClient` is true when close() caused it, false for a server or network close.
   * `code` is the close code (4001 session ended, 4003 moderator disconnect, 4004 nothing heard for SILENT_MS). Returns an
   * unsubscribe function.
   */
  onClose(cb: (byClient: boolean, code: number) => void): () => void {
    this.closeCbs.push(cb);
    return () => (this.closeCbs = this.closeCbs.filter((x) => x !== cb));
  }
}
