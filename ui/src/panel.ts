import '@fontsource-variable/inter'
import './styles/app.css'
import { mount } from 'svelte'
import Panel from './Panel.svelte'

if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) (await import('./lib/devmock')).installMock()

export default mount(Panel, { target: document.getElementById('app')! })
