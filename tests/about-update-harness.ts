import { mount } from 'svelte';
import './about-update-tauri-mock.js';
import AboutUpdateHarness from './AboutUpdateHarness.svelte';
import '../src/app.css';
mount(AboutUpdateHarness, { target: document.getElementById('app')! });
