import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
export default defineConfig({
  plugins: [
    react(),
    {
      name: 'studio-local-engine',
      configureServer(server) {
        server.middlewares.use("/__studio/workspace", (_req,res)=>{res.setHeader("Content-Type","application/json");res.end(JSON.stringify({workspace:resolve("../..")}));});
        server.middlewares.use('/__studio/connection', async (req, res) => {
          const origin = req.headers.origin;
          if (origin && !['http://127.0.0.1:1420','http://localhost:1420'].includes(origin)) {res.statusCode=403;res.end();return;}
          try {
            const path=resolve(process.env.STUDIO_DATA_DIR ?? '../../.local/dev','engine.json');
            res.setHeader('Content-Type','application/json');res.setHeader('Cache-Control','no-store');
            res.end(await readFile(path));
          } catch {res.statusCode=503;res.end('本机引擎尚未启动');}
        });
      }
    }
  ],
  server: {port:1420,strictPort:true,host:'127.0.0.1'},
  build: {target:'es2022',sourcemap:true},
  clearScreen:false
});
