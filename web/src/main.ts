import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { preloadDenoiserAssets } from './lib/audio/preload';
import { applyScheme, loadScheme, watchScheme } from './lib/ui/scheme';
import { watchTips } from './lib/ui/tip';
import { navigate } from './lib/session.svelte';

let storage: Storage | null = null;
try {
  storage = localStorage;
} catch {
  // storage blocked: nothing kept between visits
}
try {
  storage?.removeItem('vesper.style'); // the old Style choice (Persona, Cozy, BBS, Aurora): only Vesper is left
} catch {
  // storage blocked
}
watchScheme(storage);
watchTips();

// A push notification was clicked while a Vesper tab is open (public/sw.js): open the room here, so a call goes on.
navigator.serviceWorker?.addEventListener('message', (e) => {
  const url: unknown = e.data?.open;
  if (typeof url === 'string' && /^\/(r\/\d+)?$/.test(url)) navigate(url);
});

applyScheme(loadScheme(storage)); // also paints the last account's accent, so sign in shows it; App paints the real one from /api/me
mount(App, { target: document.getElementById('app')! });

// Start the denoiser download (about 10 MB) while the sign in or join form is shown.
void preloadDenoiserAssets();
