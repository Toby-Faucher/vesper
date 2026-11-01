import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { preloadDenoiserAssets } from './lib/audio/preload';

mount(App, { target: document.getElementById('app')! });

// Start the 23 MB denoiser download while the sign in or join form is shown.
void preloadDenoiserAssets();
