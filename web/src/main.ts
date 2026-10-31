import { mount } from 'svelte';
import './app.css';
import Room from './routes/Room.svelte';
import { preloadDenoiserAssets } from './lib/audio/preload';

mount(Room, { target: document.getElementById('app')! });

// Start the 23 MB denoiser download while the join form is shown.
void preloadDenoiserAssets();
