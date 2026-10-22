import { mount } from 'svelte';
import App from './App.svelte';
import SpectrogramView from './lib/SpectrogramView.svelte';
import TaskbarLyricsView from './lib/TaskbarLyricsView.svelte';
import './app.css';
import { installPointerFocusRelease } from './lib/pointer-focus.js';

// One rule for the whole app: pointer presses do not leave keyboard focus on controls (AGENTS.md).
installPointerFocusRelease(document);

// The desktop spectrogram opens as a separate window (ADR 0013) that loads this same
// bundle with `?view=spectrogram&track=<id>`.
const params = new URLSearchParams(window.location.search);
const target = document.getElementById('app')!;

// The taskbar lyrics (ADR 0018) are another window of the same bundle: `?view=taskbar-lyrics`.
const view = params.get('view');
const app =
  view === 'spectrogram'
    ? mount(SpectrogramView, { target, props: { trackId: params.get('track') ?? '' } })
    : view === 'taskbar-lyrics'
      ? mount(TaskbarLyricsView, { target })
      : mount(App, { target });

export default app;
