import { mount } from 'svelte';
import './favorite-dock-tauri-mock.js';
import FavoriteDockHarness from './FavoriteDockHarness.svelte';
import '../src/app.css';

mount(FavoriteDockHarness, { target: document.getElementById('app')! });
