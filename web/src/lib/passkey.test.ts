import { describe, expect, it } from 'vitest';
import { assertionJson, attestationJson, creationOptions, passkeysSupported, requestOptions } from './passkey';
import { toBase64Url } from './base64url';

const bytes = (...n: number[]) => new Uint8Array(n);
const buf = (...n: number[]) => bytes(...n).buffer;

describe('passkey options', () => {
  it('decodes the byte fields of a creation request and keeps the rest', () => {
    const o = creationOptions({
      publicKey: {
        rp: { name: 'Vesper', id: 'localhost' },
        user: { id: toBase64Url(bytes(1, 2, 3)), name: 'alice', displayName: 'Alice' },
        challenge: toBase64Url(bytes(251, 255)),
        excludeCredentials: [{ type: 'public-key', id: toBase64Url(bytes(9)) }],
      },
    });
    expect([...(o.challenge as Uint8Array)]).toEqual([251, 255]);
    expect([...(o.user.id as Uint8Array)]).toEqual([1, 2, 3]);
    expect([...(o.excludeCredentials![0].id as Uint8Array)]).toEqual([9]);
    expect(o.rp.id).toBe('localhost');
  });

  it('decodes a sign in request and leaves mediation out', () => {
    const o = requestOptions({ mediation: 'conditional', publicKey: { challenge: toBase64Url(bytes(7)), rpId: 'localhost', allowCredentials: [] } });
    expect([...(o.challenge as Uint8Array)]).toEqual([7]);
    expect(o.rpId).toBe('localhost');
    expect('mediation' in o).toBe(false);
  });
});

describe('passkey answers', () => {
  it('encodes an attestation without transports or extension results', () => {
    const j = attestationJson({ id: 'abc', rawId: buf(1), response: { attestationObject: buf(2), clientDataJSON: buf(3) } });
    expect(j).toEqual({ id: 'abc', rawId: 'AQ', type: 'public-key', response: { attestationObject: 'Ag', clientDataJSON: 'Aw' } });
  });

  it('encodes an assertion, with a null user handle when there is none', () => {
    const r = { authenticatorData: buf(1), clientDataJSON: buf(2), signature: buf(3) };
    const j = assertionJson({ id: 'abc', rawId: buf(1), response: { ...r, userHandle: null } }) as { response: { userHandle: unknown } };
    expect(j.response.userHandle).toBeNull();
    const k = assertionJson({ id: 'abc', rawId: buf(1), response: { ...r, userHandle: buf(4) } }) as { response: { userHandle: unknown } };
    expect(k.response.userHandle).toBe('BA');
  });

  it('reports no support without PublicKeyCredential', () => {
    expect(passkeysSupported()).toBe(false);
  });
});
