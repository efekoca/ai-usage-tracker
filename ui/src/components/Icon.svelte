<script lang="ts" module>
  // Line icons on a 24px grid, drawn to sit next to Inter at 1.6px stroke.
  const paths: Record<string, string> = {
    overview: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
    daily: 'M5 5h14a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM4 10h16M8 3v4M16 3v4',
    breakdown: 'M12 3a9 9 0 1 0 9 9h-9zM14 2.3A8 8 0 0 1 21.7 10H14z',
    limits: 'M4.5 17a8.5 8.5 0 1 1 15 0M12 13l3.5-4.5M12 13.2a1 1 0 1 0 0.01 0',
    projects: 'M3.5 6.5a1.5 1.5 0 0 1 1.5-1.5h4l2 2h8a1.5 1.5 0 0 1 1.5 1.5v9a1.5 1.5 0 0 1-1.5 1.5H5a1.5 1.5 0 0 1-1.5-1.5z',
    sources: 'M12 3c4.4 0 8 1.3 8 3s-3.6 3-8 3-8-1.3-8-3 3.6-3 8-3zM4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6',
    settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM19.4 13.5l1.6 1.2-2 3.4-1.9-.6a7 7 0 0 1-1.7 1l-.4 2h-4l-.4-2a7 7 0 0 1-1.7-1l-1.9.6-2-3.4 1.6-1.2a7 7 0 0 1 0-3L3 9.3l2-3.4 1.9.6a7 7 0 0 1 1.7-1l.4-2h4l.4 2a7 7 0 0 1 1.7 1l1.9-.6 2 3.4-1.6 1.2a7 7 0 0 1 0 3z',
    refresh: 'M20 11a8 8 0 0 0-14.3-4.3M4 5v4h4M4 13a8 8 0 0 0 14.3 4.3M20 19v-4h-4',
    exact: 'M12 2.8l2.1 1.6 2.6-.2.8 2.5 2.2 1.4-.8 2.5.8 2.5-2.2 1.4-.8 2.5-2.6-.2L12 18.4l-2.1-1.6-2.6.2-.8-2.5-2.2-1.4.8-2.5-.8-2.5 2.2-1.4.8-2.5 2.6.2zM8.6 10.6l2.3 2.3 4.5-4.6',
    estimated: 'M5 10c1.4-1.6 2.8-1.6 4.2 0s2.8 1.6 4.2 0 2.8-1.6 4.2 0M5 15c1.4-1.6 2.8-1.6 4.2 0s2.8 1.6 4.2 0 2.8-1.6 4.2 0',
    captured: 'M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10zM12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z',
    info: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 11v5M12 7.6v.4',
    warning: 'M12 4l9 16H3zM12 10v4M12 16.8v.4',
    eyeOff: 'M3 3l18 18M10.6 6.1A10 10 0 0 1 12 6c5 0 9 6 9 6a17 17 0 0 1-3 3.4M6.6 6.6A16 16 0 0 0 3 12s4 6 9 6a9 9 0 0 0 4-1M9.9 9.9a3 3 0 0 0 4.2 4.2',
    eye: 'M3 12s4-6 9-6 9 6 9 6-4 6-9 6-9-6-9-6zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
    download: 'M12 4v11M7 10l5 5 5-5M5 20h14',
    upload: 'M12 16V5M7 10l5-5 5 5M5 20h14',
    trash: 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6',
    folder: 'M3.5 6.5a1.5 1.5 0 0 1 1.5-1.5h4l2 2h8a1.5 1.5 0 0 1 1.5 1.5v9a1.5 1.5 0 0 1-1.5 1.5H5a1.5 1.5 0 0 1-1.5-1.5z',
    external: 'M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5',
    chevron: 'M9 5l7 7-7 7',
    close: 'M6 6l12 12M18 6L6 18',
    plus: 'M12 5v14M5 12h14',
    up: 'M12 19V5M6 11l6-6 6 6',
    down: 'M12 5v14M6 13l6 6 6-6',
    clock: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2',
    table: 'M4 5h16v14H4zM4 10h16M4 15h16M10 5v14',
    chart: 'M4 19h16M7 16v-4M12 16V8M17 16v-7',
    check: 'M5 12.5l4.5 4.5L19 7.5',
    power: 'M12 3v8M6.3 6.3a8 8 0 1 0 11.4 0',
    lock: 'M7 10V8a5 5 0 0 1 10 0v2M5.5 10h13v10h-13z',
    filter: 'M4 5h16l-6 7.5V19l-4-2v-4.5z',
  }
</script>

<script lang="ts">
  let { name, size = 18, label = '' }: { name: string; size?: number; label?: string } = $props()
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.6"
  stroke-linecap="round"
  stroke-linejoin="round"
  role={label ? 'img' : undefined}
  aria-label={label || undefined}
  aria-hidden={label ? undefined : 'true'}
  style="flex:none"
>
  <path d={paths[name] ?? ''} />
</svg>
