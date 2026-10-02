import '@fontsource-variable/inter'
import './styles/app.css'
import { mount } from 'svelte'
import App from './App.svelte'

// In a plain browser during development, run against a synthetic IPC mock.
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) (await import('./lib/devmock')).installMock()

// Keep the native browser context menu out of the dashboard (it offers reload/inspect).
if (!import.meta.env.DEV) window.addEventListener('contextmenu', (e) => e.preventDefault())

export default mount(App, { target: document.getElementById('app')! })
