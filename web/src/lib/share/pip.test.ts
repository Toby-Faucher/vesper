import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// No DOM here: a fake document with a video that is an EventTarget.
class FakeVideo extends EventTarget {
  srcObject: unknown = null;
  readyState = 0;
  muted = false;
  autoplay = false;
  playsInline = false;
  style = {};
  requestPictureInPicture = vi.fn(async () => {});
  play = vi.fn(async () => {});
}
let videos: FakeVideo[];
let appended: unknown[];
let pip: typeof import('./pip');

beforeEach(async () => {
  videos = [];
  appended = [];
  vi.stubGlobal('document', {
    createElement: () => {
      const v = new FakeVideo();
      videos.push(v);
      return v;
    },
    body: { append: (n: unknown) => appended.push(n) },
    pictureInPictureElement: null,
    exitPictureInPicture: vi.fn(async () => {}),
  });
  vi.resetModules();
  pip = await import('./pip'); // the element is module state: a fresh module per test
});
afterEach(() => vi.unstubAllGlobals());

const doc = (v: unknown) => ({ pictureInPictureEnabled: v }) as unknown as Document;
const stream = (id: string) => ({ id }) as unknown as MediaStream;

/** Starts popOut and lets the video load its metadata. */
async function pop(s: MediaStream, onClose = () => {}) {
  const p = pip.popOut(s, onClose);
  videos[0].dispatchEvent(new Event('loadedmetadata'));
  await p;
}

describe('canPopOut', () => {
  it('is false without the API (Firefox) or when it is off', () => {
    expect(pip.canPopOut(doc(undefined))).toBe(false);
    expect(pip.canPopOut(doc(false))).toBe(false);
  });
  it('is true when the browser has it on', () => {
    expect(pip.canPopOut(doc(true))).toBe(true);
  });
});

describe('popOut', () => {
  it('waits for the metadata, then asks for the window', async () => {
    const s = stream('a');
    const p = pip.popOut(s, () => {});
    await Promise.resolve();
    expect(videos[0].srcObject).toBe(s);
    expect(videos[0].requestPictureInPicture).not.toHaveBeenCalled();
    videos[0].dispatchEvent(new Event('loadedmetadata'));
    await p;
    expect(videos[0].requestPictureInPicture).toHaveBeenCalledOnce();
  });
  it('reuses one element across calls', async () => {
    await pop(stream('a'));
    await pop(stream('b'));
    expect(videos).toHaveLength(1);
    expect(appended).toEqual([videos[0]]);
    expect(videos[0].srcObject).toEqual(stream('b'));
    expect(videos[0].requestPictureInPicture).toHaveBeenCalledTimes(2);
  });
  it('calls onClose when the window closes', async () => {
    const onClose = vi.fn();
    await pop(stream('a'), onClose);
    expect(onClose).not.toHaveBeenCalled();
    videos[0].dispatchEvent(new Event('leavepictureinpicture'));
    expect(onClose).toHaveBeenCalledOnce();
  });
  it('calls onClose when the browser refuses the window', async () => {
    const onClose = vi.fn();
    const p = pip.popOut(stream('a'), onClose);
    videos[0].requestPictureInPicture.mockRejectedValueOnce(new DOMException('no gesture', 'NotAllowedError'));
    videos[0].dispatchEvent(new Event('loadedmetadata'));
    await p;
    expect(onClose).toHaveBeenCalledOnce();
  });
  it('a newer pop-out replaces the onClose of the one before', async () => {
    const first = vi.fn(), second = vi.fn();
    await pop(stream('a'), first);
    await pop(stream('b'), second);
    videos[0].dispatchEvent(new Event('leavepictureinpicture'));
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledOnce();
  });
  it('asks for no window when closed while it waited for the metadata', async () => {
    const p = pip.popOut(stream('a'), () => {});
    pip.closePopOut();
    videos[0].dispatchEvent(new Event('loadedmetadata'));
    await p;
    expect(videos[0].requestPictureInPicture).not.toHaveBeenCalled();
  });
});

describe('closePopOut', () => {
  it('does nothing before any pop-out', () => {
    pip.closePopOut();
    expect(document.exitPictureInPicture).not.toHaveBeenCalled();
  });
  it('closes the window when ours is open and calls onClose once', async () => {
    const onClose = vi.fn();
    await pop(stream('a'), onClose);
    (document as unknown as { pictureInPictureElement: unknown }).pictureInPictureElement = videos[0];
    pip.closePopOut();
    expect(document.exitPictureInPicture).toHaveBeenCalledOnce();
    expect(videos[0].srcObject).toBeNull();
    videos[0].dispatchEvent(new Event('leavepictureinpicture'));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
