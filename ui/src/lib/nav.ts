import type { View } from './store.svelte'

/** The sidebar's pages, which the command palette lists too. */
export const NAV: { section: string; items: { id: View; icon: string }[] }[] = [
  { section: 'nav.section.usage', items: [
    { id: 'overview', icon: 'overview' },
    { id: 'tips', icon: 'tips' },
    { id: 'daily', icon: 'daily' },
    { id: 'breakdown', icon: 'breakdown' },
    { id: 'sessions', icon: 'sessions' },
    { id: 'cache', icon: 'cache' },
    { id: 'context', icon: 'context' },
    { id: 'limits', icon: 'limits' },
    { id: 'projects', icon: 'projects' },
  ] },
  { section: 'nav.section.manage', items: [
    { id: 'sources', icon: 'sources' },
    { id: 'widget', icon: 'widget' },
    { id: 'menubar', icon: 'menubar' },
    { id: 'settings', icon: 'settings' },
  ] },
]
