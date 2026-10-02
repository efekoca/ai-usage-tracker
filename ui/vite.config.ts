import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'
import { resolve } from 'node:path'

// Two entry pages: the dashboard and the always-on-top widget.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // fs.allow lets the dev-only mock read ../config/plans.json
  server: { port: 5173, strictPort: true, fs: { allow: ['..'] } },
  build: {
    target: 'chrome120',
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, 'index.html'),
        widget: resolve(import.meta.dirname, 'widget.html'),
      },
    },
  },
})
