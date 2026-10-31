import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Relative base so the app works at "/" or behind a reverse proxy sub-path.
export default defineConfig({
  base: "./",
  plugins: [react()],
  build: { outDir: "dist", assetsDir: "assets", sourcemap: false, chunkSizeWarningLimit: 700 },
  server: {
    proxy: { "/api": process.env.AIRSLICE_BACKEND ?? "http://127.0.0.1:8080" },
  },
});
