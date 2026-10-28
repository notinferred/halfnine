import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Tauri opens the window on this exact port (crates/desktop/tauri.conf.json), so Vite must not pick another.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "safari16", outDir: "dist", emptyOutDir: true },
});
