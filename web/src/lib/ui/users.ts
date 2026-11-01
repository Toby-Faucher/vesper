import { api } from '../api';
import type { UserInfo } from '../gen/UserInfo';
import { session } from '../session.svelte';

// Someone who signed up after this tab connected is not in the hello's user list (no event announces them yet): ask the
// server and keep the answer in the session, so every later card and the switcher have it. One request per signed-in
// account per FRESH_MS; a failure is retried on the next call.
// shortcut: a signup shows on the next open after FRESH_MS at worst; a server signup event is the real fix (known-issues.md).
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
