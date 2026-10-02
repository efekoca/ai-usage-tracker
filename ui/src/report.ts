import '@fontsource-variable/inter'
import './styles/app.css'
import { mount } from 'svelte'
import Report from './Report.svelte'

// In a plain browser during development, run against a synthetic IPC mock.
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) (await import('./lib/devmock')).installMock()

export default mount(Report, { target: document.getElementById('app')! })
