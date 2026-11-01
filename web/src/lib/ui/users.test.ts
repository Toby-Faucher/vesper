import { afterEach, expect, it, vi } from 'vitest';

const user = (id: string) => ({ user_id: id, username: id, display_name: id, role: 'member', banned: false, accent: '' });

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('asks_once_then_again_after_fresh_ms', async () => {
  vi.useFakeTimers();
  let list = [user('a')];
  const fetchMock = vi.fn(async () => new Response(JSON.stringify(list), { status: 200 }));
  vi.stubGlobal('fetch', fetchMock);
  vi.stubGlobal('location', { pathname: '/', search: '', hash: '' }); // read when the session module loads
  const { FRESH_MS, loadUsers } = await import('./users');
  const { session } = await import('../session.svelte');
  await loadUsers();
  await loadUsers();
  expect(fetchMock).toHaveBeenCalledTimes(1);
  list = [user('a'), user('late')]; // signed up after the first answer
  vi.advanceTimersByTime(FRESH_MS + 1);
  await loadUsers();
  expect(fetchMock).toHaveBeenCalledTimes(2);
  expect(session.state.users.late?.display_name).toBe('late');
});
