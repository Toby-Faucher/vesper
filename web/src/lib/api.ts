import type { ApiErrorCode } from './gen/ApiErrorCode';

export class ApiFailure extends Error {
  constructor(
    public status: number,
    public code: ApiErrorCode,
  ) {
    super(`${status} ${code}`);
  }
}

/** JSON request to the server's `/api`. Rejects with ApiFailure on an error status; 204 resolves undefined. */
export async function api<T>(method: 'GET' | 'POST' | 'PATCH' | 'PUT' | 'DELETE', path: string, body?: unknown): Promise<T> {
  const r = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!r.ok) {
    const code = await r.json().then((e) => e?.code, () => undefined);
    throw new ApiFailure(r.status, typeof code === 'string' ? (code as ApiErrorCode) : 'internal');
  }
  return r.status === 204 ? (undefined as T) : r.json();
}

/** Plain text for an API error code. */
export const API_MESSAGES: Record<ApiErrorCode, string> = {
  bad_request: 'The server did not accept that. Check what you typed.',
  unauthorized: 'You are signed out. Sign in again.',
  forbidden: 'You are not allowed to do that.',
  not_found: 'That was not found.',
  conflict: 'That name is already taken.',
  rate_limited: 'Too many tries. Wait a minute and try again.',
  too_large: 'That is too large.',
  quota_full: 'The server is out of space for uploads.',
  invite_invalid: 'This invite link is not valid. Ask for a new one.',
  internal: 'Something went wrong on the server. Try again.',
};

/** A message for any error from `api`: ApiFailure codes map through the table, anything else is a network failure. */
export function errorText(e: unknown): string {
  return e instanceof ApiFailure ? (API_MESSAGES[e.code] ?? API_MESSAGES.internal) : 'Could not reach the server. Check your connection.';
}
