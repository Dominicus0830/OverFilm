import {defineConfig} from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';

// 렌더 템플릿(../render/src)을 그대로 가져와 미리보기에 쓴다. remotion·react 는 한 벌만.
const here = path.dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'));
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      remotion: path.resolve(here, 'node_modules/remotion'),
      react: path.resolve(here, 'node_modules/react'),
      'react-dom': path.resolve(here, 'node_modules/react-dom'),
    },
    dedupe: ['remotion', 'react', 'react-dom'],
  },
  server: {
    fs: {allow: ['..']},
    proxy: {'/api': 'http://127.0.0.1:8765', '/files': 'http://127.0.0.1:8765',
      '/ws': {target: 'ws://127.0.0.1:8765', ws: true}},
  },
});
