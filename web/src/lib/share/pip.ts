// The pop-out: one hidden <video> on document.body, not in the room page, so a popped-out share outlives that page.
let el: HTMLVideoElement | null = null;
let closed: (() => void) | null = null;

/** Firefox has no requestPictureInPicture (its own pop-out button works instead). */
export const canPopOut = (doc: Document = document): boolean => doc.pictureInPictureEnabled === true;

/** Runs the pending onClose once. */
function ended() {
  const f = closed;
  closed = null;
  f?.();
}

/** Call it from the click handler: the browser asks for a user gesture. onClose runs once when the window closes or cannot open. */
export async function popOut(stream: MediaStream, onClose: () => void): Promise<void> {
  if (!el) {
    const v = (el = document.createElement('video'));
    Object.assign(v, { muted: true, autoplay: true, playsInline: true });
    // Drawn but not seen: a video with display none may not be allowed into picture-in-picture.
    Object.assign(v.style, { position: 'fixed', width: '1px', height: '1px', opacity: '0', pointerEvents: 'none' });
    v.addEventListener('leavepictureinpicture', () => {
      v.srcObject = null;
      ended();
    });
    document.body.append(v);
  }
  const v = el;
  closed = onClose; // another share already out: it swaps in, in the same window
  if (v.srcObject !== stream) v.srcObject = stream;
  void v.play().catch(() => {});
  if (v.readyState < 1) await new Promise((r) => v.addEventListener('loadedmetadata', r, { once: true }));
  if (v.srcObject !== stream) return; // closed, or another share popped out, while it loaded
  await v.requestPictureInPicture().catch(() => {
    // the gesture ran out, or the browser said no: the hidden video stops drawing the share
    if (v.srcObject === stream) v.srcObject = null; // not a newer pop-out's
    if (closed === onClose) ended();
  });
}

/** Closes the window, if any, and runs the pending onClose. */
export function closePopOut(): void {
  if (!el) return;
  el.srcObject = null;
  if (document.pictureInPictureElement === el) void document.exitPictureInPicture().catch(() => {});
  ended();
}
