import { expect, it } from 'vitest';
import { ApiFailure, API_MESSAGES } from '../../lib/api';
import { UI_MESSAGES as M } from '../../lib/ui/messages';
import { pwCheck, pwFailure } from './account';

it('password_checks_before_any_request', () => {
  expect(pwCheck('short', 'short')).toEqual({ at: 'new', text: M.passwordShort });
  expect(pwCheck('a'.repeat(129), 'a'.repeat(129))?.at).toBe('new');
  // Counted in characters like the server: 8 emoji are 16 UTF-16 units and still fine; 129 are too many.
  expect(pwCheck('😀'.repeat(8), '😀'.repeat(8))).toBeNull();
  expect(pwCheck('😀'.repeat(65), '😀'.repeat(65))).toBeNull();
  expect(pwCheck('😀'.repeat(129), '😀'.repeat(129))?.at).toBe('new');
  expect(pwCheck('test-password-2', 'test-password-3')).toEqual({ at: 'confirm', text: M.passwordMismatch });
  expect(pwCheck('test-password-2', 'test-password-2')).toBeNull();
});

it('only_a_wrong_password_maps_to_the_current_field', () => {
  expect(pwFailure(new ApiFailure(403, 'forbidden', 'wrong password'))).toEqual({ at: 'old', text: M.passwordWrong });
  // The Origin check is a 403 forbidden too, with another message.
  expect(pwFailure(new ApiFailure(403, 'forbidden', 'request from another origin'))).toEqual({ at: 'form', text: API_MESSAGES.forbidden });
  expect(pwFailure(new ApiFailure(429, 'rate_limited'))).toEqual({ at: 'form', text: API_MESSAGES.rate_limited });
  expect(pwFailure(new TypeError('Failed to fetch')).at).toBe('form');
});
