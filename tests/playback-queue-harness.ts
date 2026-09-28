import { mount } from 'svelte';
import PlaybackQueueHarness from './PlaybackQueueHarness.svelte';
import '../src/app.css';

mount(PlaybackQueueHarness, { target: document.querySelector('#app')! });
