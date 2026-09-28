import { mount } from 'svelte';
import LyricsViewHarness from './LyricsViewHarness.svelte';
import '../src/app.css';

mount(LyricsViewHarness, { target: document.getElementById('app')! });
