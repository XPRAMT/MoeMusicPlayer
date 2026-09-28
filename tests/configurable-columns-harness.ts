import { mount } from 'svelte';
import ConfigurableColumnsHarness from './ConfigurableColumnsHarness.svelte';
import '../src/app.css';

mount(ConfigurableColumnsHarness, { target: document.querySelector('#app')! });
