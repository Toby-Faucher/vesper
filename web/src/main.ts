import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { preloadDenoiserAssets } from './lib/audio/preload';
import { applyAccent, loadAccent } from './lib/ui/accent';
import { applyStyle, loadStyle } from './lib/ui/style';
import { watchTips } from './lib/ui/tip';
import { navigate } from './lib/session.svelte';

let storage: Storage | null = null;
try {
  storage = localStorage;
} catch {
  // storage blocked: the default style
}
applyStyle(loadStyle(storage));
applyAccent(loadAccent()); // the last account's accent, so sign in shows it; App paints the real one from /api/me
watchTips();

// A push notification was clicked while a Vesper tab is open (public/sw.js): open the room here, so a call goes on.
navigator.serviceWorker?.addEventListener('message', (e) => {
  const url: unknown = e.data?.open;
  if (typeof url === 'string' && /^\/(r\/\d+)?$/.test(url)) navigate(url);
});

mount(App, { target: document.getElementById('app')! });

// Start the 23 MB denoiser download while the sign in or join form is shown.
void preloadDenoiserAssets();
