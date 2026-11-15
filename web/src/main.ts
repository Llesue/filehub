import '@fontsource/noto-sans/latin-400.css';
import '@fontsource/noto-sans/latin-500.css';
import '@fontsource/noto-sans/latin-600.css';
import '@fontsource/noto-sans-mono/latin-400.css';
import './lib/styles/tokens.css';
import './lib/styles/base.css';

import { mount } from 'svelte';
import App from './App.svelte';

const target = document.getElementById('app');

if (!target) {
  throw new Error('missing #app');
}

mount(App, { target });
