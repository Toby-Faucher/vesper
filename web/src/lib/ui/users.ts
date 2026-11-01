import { api } from '../api';
import type { UserInfo } from '../gen/UserInfo';
import { session } from '../session.svelte';

// Someone who signed up after this tab connected is not in the hello's user list (no event announces them yet): ask the
// server once per signed-in account and keep the answer in the session, so every later card and the switcher have it.
// A failure is retried on the next call.
let asked: { by: string; p: Promise<void> } | null = null;
export function loadUsers(): Promise<void> {
  const by = session.state.me?.user.user_id ?? '';
  if (asked?.by !== by) {
    const p = api<UserInfo[]>('GET', '/api/users').then(
      (l) => { session.state.users = { ...Object.fromEntries(l.map((u) => [u.user_id, u])), ...session.state.users }; },
      (e) => { asked = null; throw e; },
    );
    asked = { by, p };
  }
  return asked.p;
}
