import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { readFileSync } from 'node:fs'

const tauriConfig = JSON.parse(readFileSync(new URL('./src-tauri/tauri.conf.json', import.meta.url), 'utf-8')) as {
  version?: string
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(tauriConfig.version ?? '0.0.0'),
    __GITHUB_RELEASES_REPO__: JSON.stringify(process.env.VITE_GITHUB_RELEASES_REPO ?? ''),
  },
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
  },
})
