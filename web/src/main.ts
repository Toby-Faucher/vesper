import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { preloadDenoiserAssets } from './lib/audio/preload';
import { applyScheme, loadScheme, watchScheme } from './lib/ui/scheme';
import { applyStyle, loadStyle, useStyle } from './lib/ui/style';
import { watchTips } from './lib/ui/tip';
import { navigate } from './lib/session.svelte';

let storage: Storage | null = null;
try {
  storage = localStorage;
} catch {
  // storage blocked: the default style
}
const style = loadStyle(storage);
// A lazy style loads its CSS first (at most 2 s), so the page never shows it without its tokens.
const ready = style === 'vesper' ? Promise.resolve(applyStyle(style)) : Promise.race([useStyle(style), new Promise((r) => setTimeout(r, 2000))]);
watchScheme(storage);
watchTips();

// A push notification was clicked while a Vesper tab is open (public/sw.js): open the room here, so a call goes on.
navigator.serviceWorker?.addEventListener('message', (e) => {
  const url: unknown = e.data?.open;
  if (typeof url === 'string' && /^\/(r\/\d+)?$/.test(url)) navigate(url);
});

void ready.then(() => {
  applyScheme(loadScheme(storage)); // also paints the last account's accent, so sign in shows it; App paints the real one from /api/me
  mount(App, { target: document.getElementById('app')! });
});

// Start the denoiser download (about 10 MB) while the sign in or join form is shown.
void preloadDenoiserAssets();
