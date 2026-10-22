import { mount } from 'svelte';
import App from './App.svelte';
import SpectrogramView from './lib/SpectrogramView.svelte';
import TaskbarLyricsView from './lib/TaskbarLyricsView.svelte';
import FloatingLyricsView from './lib/FloatingLyricsView.svelte';
import './app.css';
import { installPointerFocusRelease } from './lib/pointer-focus.js';

// One rule for the whole app: pointer presses do not leave keyboard focus on controls (AGENTS.md).
installPointerFocusRelease(document);

// The desktop spectrogram opens as a separate window (ADR 0013) that loads this same
// bundle with `?view=spectrogram&track=<id>`.
const params = new URLSearchParams(window.location.search);
const target = document.getElementById('app')!;

// The taskbar lyrics (ADR 0018) and the floating lyrics (ADR 0032) are other windows of the same bundle.
const view = params.get('view');
const app =
  view === 'spectrogram'
    ? mount(SpectrogramView, { target, props: { trackId: params.get('track') ?? '' } })
    : view === 'taskbar-lyrics'
      ? mount(TaskbarLyricsView, { target })
      : view === 'floating-lyrics'
        ? mount(FloatingLyricsView, { target })
        : mount(App, { target });

export default app;
