import { mount } from 'svelte';
import QuickIndexHarness from './QuickIndexHarness.svelte';
import '../src/app.css';

mount(QuickIndexHarness, { target: document.querySelector('#app')! });
