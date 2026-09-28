import { mount } from 'svelte';
import './volume-slider-tauri-mock.js';
import App from '../src/App.svelte';
import '../src/app.css';

mount(App, { target: document.getElementById('app')! });
