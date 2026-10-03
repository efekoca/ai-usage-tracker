import '@fontsource-variable/inter'
import './styles/app.css'
import { mount } from 'svelte'
import App from './App.svelte'

if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) (await import('./lib/devmock')).installMock()

// the native context menu offers reload/inspect
if (!import.meta.env.DEV) window.addEventListener('contextmenu', (e) => e.preventDefault())

export default mount(App, { target: document.getElementById('app')! })
