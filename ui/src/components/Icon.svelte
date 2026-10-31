<script lang="ts" module>
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
    cache: 'M12 3l9 4.5-9 4.5-9-4.5zM3 12l9 4.5 9-4.5M3 16.5l9 4.5 9-4.5',
    widget: 'M4.5 6.5a2 2 0 0 1 2-2h11a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-11a2 2 0 0 1-2-2zM8 18h8M12 13.5V18',
    menubar: 'M3 4h18v4.5H3zM14 6.25h4M12 11h8v8.5h-8zM14.5 14h3M14.5 16.5h3',
    sessions: 'M9 6h11M9 12h11M9 18h11M4.5 5.2v1.6M4.5 11.2v1.6M4.5 17.2v1.6',
    context: 'M4 8.5V5a1 1 0 0 1 1-1h3.5M15.5 4H19a1 1 0 0 1 1 1v3.5M20 15.5V19a1 1 0 0 1-1 1h-3.5M8.5 20H5a1 1 0 0 1-1-1v-3.5M8 12h8M8 9h5M8 15h6',
    report: 'M7 3h7l5 5v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM14 3v5h5M9.5 13h6M9.5 17h4',
    search: 'M11 4.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM20 20l-4.4-4.4',
    tips: 'M9.5 18h5M10.5 21h3M12 3a6 6 0 0 0-3.6 10.8c.7.5 1.1 1.3 1.1 2.1v.1h5v-.1c0-.8.4-1.6 1.1-2.1A6 6 0 0 0 12 3z',
    keyboard: 'M3.5 7a1.5 1.5 0 0 1 1.5-1.5h14A1.5 1.5 0 0 1 20.5 7v10a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 17zM7 10h.01M10 10h.01M13 10h.01M16 10h.01M8 14.5h8',
  }

  // filled marks for the tray panel's buttons; a part with a width is stroked, the rest filled
  const solid: Record<string, { d: string; width?: number }[]> = {
    app: [{ d: 'M6.5 4h11A3.5 3.5 0 0 1 21 7.5v9a3.5 3.5 0 0 1-3.5 3.5h-11A3.5 3.5 0 0 1 3 16.5v-9A3.5 3.5 0 0 1 6.5 4zM8.5 12.25a.75.75 0 0 0-.75.75v3a.75.75 0 0 0 1.5 0v-3a.75.75 0 0 0-.75-.75zM12 8.75a.75.75 0 0 0-.75.75v6.5a.75.75 0 0 0 1.5 0V9.5a.75.75 0 0 0-.75-.75zM15.5 10.75a.75.75 0 0 0-.75.75v4.5a.75.75 0 0 0 1.5 0v-4.5a.75.75 0 0 0-.75-.75z' }],
    pip: [
      { d: 'M6 5h12a2.5 2.5 0 0 1 2.5 2.5v9A2.5 2.5 0 0 1 18 19H6a2.5 2.5 0 0 1-2.5-2.5v-9A2.5 2.5 0 0 1 6 5z', width: 1.75 },
      { d: 'M13 11.75h4a1.25 1.25 0 0 1 1.25 1.25v2.25a1.25 1.25 0 0 1-1.25 1.25h-4a1.25 1.25 0 0 1-1.25-1.25V13A1.25 1.25 0 0 1 13 11.75z' },
    ],
    sync: [{ d: 'M15.5 5.94A7 7 0 1 1 12 5', width: 2.25 }, { d: 'M10.75 1.75L15 5l-4.25 3.25z' }],
    gear: [
      {
        d: 'M10 4.77L10.31 2.4L13.69 2.4L14 4.77A7.5 7.5 0 0 1 15.69 5.47L17.59 4.01L19.99 6.41L18.53 8.31A7.5 7.5 0 0 1 19.23 10L21.6 10.31L21.6 13.69L19.23 14A7.5 7.5 0 0 1 18.53 15.69L19.99 17.59L17.59 19.99L15.69 18.53A7.5 7.5 0 0 1 14 19.23L13.69 21.6L10.31 21.6L10 19.23A7.5 7.5 0 0 1 8.31 18.53L6.41 19.99L4.01 17.59L5.47 15.69A7.5 7.5 0 0 1 4.77 14L2.4 13.69L2.4 10.31L4.77 10A7.5 7.5 0 0 1 5.47 8.31L4.01 6.41L6.41 4.01L8.31 5.47A7.5 7.5 0 0 1 10 4.77ZM15.25 12a3.25 3.25 0 1 1-6.5 0 3.25 3.25 0 0 1 6.5 0z',
      },
    ],
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
  {#if solid[name]}
    {#each solid[name] as part (part.d)}
      <path d={part.d} fill={part.width ? 'none' : 'currentColor'} fill-rule="evenodd" stroke={part.width ? 'currentColor' : 'none'} stroke-width={part.width} />
    {/each}
  {:else}
    <path d={paths[name] ?? ''} />
  {/if}
</svg>
