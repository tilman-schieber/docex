import { defineConfig } from "vite";

// Tauri expects a fixed port and doesn't need the browser opened.
export default defineConfig({
  clearScreen: false,
  // /invoke: the dev server's backend, for dev.html (browser tests, screenshots).
  server: { port: 5173, strictPort: true, proxy: { "/invoke": `http://127.0.0.1:${process.env.DOCEX_DEVSERVER_PORT || 1431}` } },
  envPrefix: ["VITE_", "TAURI_"],
  build: { target: "safari15", outDir: "dist", emptyOutDir: true },
});
