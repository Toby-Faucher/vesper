import { describe, expect, it } from 'vitest';
import { ApiFailure } from '../../lib/api';
import { passkeyFailure } from './passkeys';

describe('passkeyFailure', () => {
  it('tells a cancel from a server answer', () => {
    expect(passkeyFailure(new DOMException('x', 'NotAllowedError'))).toMatch(/Cancelled/);
    expect(passkeyFailure(new DOMException('x', 'InvalidStateError'))).toMatch(/already has a passkey/);
    expect(passkeyFailure(new DOMException('x', 'SecurityError'))).toMatch(/address/);
    expect(passkeyFailure(new ApiFailure(409, 'conflict'))).toMatch(/most passkeys/);
    expect(passkeyFailure(new ApiFailure(400, 'bad_request'))).toMatch(/could not be added/);
  });

  it('only a sign in 401 says the passkey did not work', () => {
    expect(passkeyFailure(new ApiFailure(401, 'unauthorized'), true)).toMatch(/did not work/);
    expect(passkeyFailure(new ApiFailure(401, 'unauthorized'))).toMatch(/signed out/);
    expect(passkeyFailure(new TypeError('network'))).toMatch(/Could not reach/);
  });
});
