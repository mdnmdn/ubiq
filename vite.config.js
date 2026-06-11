import { defineConfig } from 'vite';

export default defineConfig({
  // Vite options tailored for Tauri development
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // tell vite to ignore watching `src-tauri`
      ignored: ['**/src-tauri/**'],
    },
  },
});