// Copies text. The Clipboard API is missing on plain http and can refuse; then a hidden textarea and execCommand,
// the old way, which still works there. False when both failed, so the caller can say so.
export async function copyText(text: string, nav: Pick<Navigator, 'clipboard'> = navigator, doc: Document = document): Promise<boolean> {
  try {
    await nav.clipboard.writeText(text);
    return true;
  } catch {
    // missing or refused: try the old way below
  }
  const back = doc.activeElement as HTMLElement | null;
  const ta = doc.createElement('textarea');
  ta.value = text;
  ta.setAttribute('readonly', '');
  ta.style.position = 'fixed';
  ta.style.opacity = '0';
  doc.body.append(ta);
  ta.select();
  try {
    return doc.execCommand('copy');
  } catch {
    return false;
  } finally {
    ta.remove();
    back?.focus({ preventScroll: true });
  }
}
