import { describe, expect, it, vi } from 'vitest';
import { measureUploadKbps, SPEEDTEST_BYTES, uploadOnce } from './speedtest';

const clock = (...t: number[]) => () => t.shift() ?? 0;

describe('measureUploadKbps', () => {
  it('posts the bytes and returns kbps rounded down', async () => {
    let sent: Blob | null = null;
    const kbps = await measureUploadKbps(async (b) => { sent = b; return new Response(null, { status: 204 }); }, clock(0, 2000));
    expect(sent).toBeInstanceOf(Blob);
    expect(sent!.size).toBe(SPEEDTEST_BYTES);
    expect(kbps).toBe(16_777);
  });
  it('is null on 429', async () => {
    expect(await measureUploadKbps(async () => new Response(null, { status: 429 }), clock(0, 2000))).toBeNull();
  });
  it('is null when the post throws', async () => {
    expect(await measureUploadKbps(async () => { throw new Error('offline'); }, clock(0, 2000))).toBeNull();
  });
});

describe('uploadOnce', () => {
  it('measures once per page; a failure is measured again', async () => {
    const measure = vi.fn<() => Promise<number | null>>().mockResolvedValueOnce(null).mockResolvedValue(9_000);
    expect(await uploadOnce(measure)).toBeNull();
    expect(await uploadOnce(measure)).toBe(9_000);
    expect(await uploadOnce(measure)).toBe(9_000);
    expect(measure).toHaveBeenCalledTimes(2);
  });
});
