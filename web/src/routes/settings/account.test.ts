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

it('a recent call reads as plain words, with a dash for a figure never measured', async () => {
  const { callLine } = await import('./account');
  const base = { id: 1, started_at: 0, ended_at: 0, fallback_reason: '', loss_pct: 0.25, jitter_ms: 8.4, concealed_pct: null, rtt_ms: 41.6, browser_ns_loading: false } as const;
  expect(callLine({ ...base, duration_s: 45, mode: 'deepfilter' })).toEqual({
    head: '45 s, DeepFilterNet', figures: 'Loss 0.3 %, jitter 8 ms, round trip 42 ms, concealed –',
  });
  expect(callLine({ ...base, duration_s: 3900, mode: 'browser', fallback_reason: 'denoiser stalled', browser_ns_loading: true }).head)
    .toBe('1 h 5 min, browser filter (denoiser stalled), browser filter while it loaded');
  expect(callLine({ ...base, duration_s: 600, mode: 'off' }).head).toBe('10 min, no filter');
});
