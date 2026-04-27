import { createReadStream, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, join } from 'node:path';
import { fileURLToPath, URL } from 'node:url';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig, type Plugin } from 'vite';

const host = process.env.TAURI_DEV_HOST;
const captureUi = process.env.GOBCAM_CAPTURE_UI === '1';
const captureCacheRoot =
  process.env.GOBCAM_CAPTURE_CACHE_ROOT ?? join(homedir(), '.cache', 'gobcam');

function capturePreviewPlugin(): Plugin {
  return {
    name: 'gobcam-capture-preview-assets',
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const prefix = '/__gobcam-preview/';
        const rawUrl = req.url ?? '';
        if (!rawUrl.startsWith(prefix)) {
          next();
          return;
        }

        const rawFileName = rawUrl.slice(prefix.length).split(/[?#]/, 1)[0] ?? '';
        const fileName = decodeURIComponent(rawFileName);
        if (fileName !== basename(fileName) || !fileName.endsWith('.png')) {
          res.statusCode = 400;
          res.end('bad preview path');
          return;
        }

        const previewPath = join(captureCacheRoot, 'previews', fileName);
        if (!existsSync(previewPath)) {
          res.statusCode = 404;
          res.end('preview not cached');
          return;
        }

        res.setHeader('Content-Type', 'image/png');
        createReadStream(previewPath).pipe(res);
      });
    },
  };
}

export default defineConfig({
  plugins: [svelte(), tailwindcss(), ...(captureUi ? [capturePreviewPlugin()] : [])],
  clearScreen: false,
  resolve: captureUi
    ? {
        alias: {
          '@tauri-apps/api/core': fileURLToPath(
            new URL('./src/capture/tauriCore.ts', import.meta.url),
          ),
        },
      }
    : undefined,
  server: {
    port: 1420,
    strictPort: true,
    host: host ?? false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
});
