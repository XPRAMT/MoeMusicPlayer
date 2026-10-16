import { mount } from 'svelte';
import App from './App.svelte';
import SpectrogramView from './lib/SpectrogramView.svelte';
import './app.css';

// The desktop spectrogram opens as a separate window (ADR 0013) that loads this same
// bundle with `?view=spectrogram&track=<id>`.
const params = new URLSearchParams(window.location.search);
const target = document.getElementById('app')!;

const app =
  params.get('view') === 'spectrogram'
    ? mount(SpectrogramView, { target, props: { trackId: params.get('track') ?? '' } })
    : mount(App, { target });

export default app;
