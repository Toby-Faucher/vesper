import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { preloadDenoiserAssets } from './lib/audio/preload';
import { applyStyle, loadStyle } from './lib/ui/style';

let storage: Storage | null = null;
try {
  storage = localStorage;
} catch {
  // storage blocked: the default style
}
applyStyle(loadStyle(storage));

mount(App, { target: document.getElementById('app')! });

// Start the 23 MB denoiser download while the sign in or join form is shown.
void preloadDenoiserAssets();
