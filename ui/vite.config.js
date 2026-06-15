import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri очікує фіксований порт у dev і не хоче, щоб Vite чистив екран,
// інакше губляться повідомлення Rust-сторони.
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      // Не стежимо за Rust-стороною — це робить Tauri.
      ignored: ["**/src-tauri/**"],
    },
  },
  // Артефакти збірки кладемо в ui/dist (його читає Tauri як frontendDist).
  build: {
    target: "esnext",
    outDir: "dist",
    emptyOutDir: true,
  },
});
