import { mount } from 'svelte';
import './app.css';
import Room from './routes/Room.svelte';

mount(Room, { target: document.getElementById('app')! });
