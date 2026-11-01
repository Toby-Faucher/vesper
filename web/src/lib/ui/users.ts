import { api } from '../api';
import type { UserInfo } from '../gen/UserInfo';
import { session } from '../session.svelte';

// The session's user list comes from the hello and is kept current by user_created and user_updated. This asks the server
// again for the rare gap (an event lost while the socket was down and before the next hello) and keeps the answer in the
// session; what the session already has wins. One request per signed-in account per FRESH_MS; a failure is retried on
// the next call.
export const FRESH_MS = 30_000;
let asked: { by: string; at: number; p: Promise<void> } | null = null;
export function loadUsers(): Promise<void> {
  const by = session.state.me?.user.user_id ?? '';
  if (asked?.by !== by || Date.now() - asked.at > FRESH_MS) {
    const p = api<UserInfo[]>('GET', '/api/users').then(
      (l) => { session.state.users = { ...Object.fromEntries(l.map((u) => [u.user_id, u])), ...session.state.users }; },
      (e) => { asked = null; throw e; },
    );
    asked = { by, at: Date.now(), p };
  }
  return asked.p;
}
