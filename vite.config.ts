import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

// Set by `tauri dev` when the frontend is served to another device.
const host = process.env.TAURI_DEV_HOST;

// Vite only serves and bundles the frontend; `tauri dev` and `tauri build` run it.
export default defineConfig({
  // Keep Rust errors on screen.
  clearScreen: false,
  // Tauri loads the dev server from a fixed port (devUrl in tauri.conf.json).
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  // Two pages: the app's window, and the overlay (#49), which the stream page serves too (#55).
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("index.html", import.meta.url)),
        overlay: fileURLToPath(new URL("overlay.html", import.meta.url)),
      },
    },
  },
});
