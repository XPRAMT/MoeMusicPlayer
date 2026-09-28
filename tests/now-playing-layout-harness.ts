import { mount } from 'svelte';
import NowPlayingLayoutHarness from './NowPlayingLayoutHarness.svelte';
import '../src/app.css';

mount(NowPlayingLayoutHarness, { target: document.querySelector('#app')! });
