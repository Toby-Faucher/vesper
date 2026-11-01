// Settings > Account password rules, outside the component so vitest can load them without the page.
import { ApiFailure, errorText } from '../../lib/api';
import { UI_MESSAGES as M } from '../../lib/ui/messages';
export type PwError = { at: 'old' | 'new' | 'confirm' | 'form'; text: string };
/** The checks before any request: the length (in characters, as the server counts) and the two new passwords match. */
export function pwCheck(newPw: string, confirmPw: string): PwError | null {
  const n = [...newPw].length;
  if (n < 8 || n > 128) return { at: 'new', text: M.passwordShort };
  return newPw === confirmPw ? null : { at: 'confirm', text: M.passwordMismatch };
}
/** A failed change. The server sends 403 forbidden for a wrong current password and for a request from another
 *  origin; only its message tells them apart. */
export const pwFailure = (err: unknown): PwError =>
  err instanceof ApiFailure && err.code === 'forbidden' && err.detail === 'wrong password' ? { at: 'old', text: M.passwordWrong } : { at: 'form', text: errorText(err) };
