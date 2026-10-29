import { defineConfig } from "vite";

// Tauri expects a fixed port and doesn't need the browser opened.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  envPrefix: ["VITE_", "TAURI_"],
  build: { target: "safari15", outDir: "dist", emptyOutDir: true },
});
