import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The app is served to a WebView, never to a public web server: the build is a
// plain static bundle that `src-tauri` embeds, and `tauri dev` expects the dev
// server on this exact port.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  // Keeps the Rust compiler output readable while `tauri dev` runs.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // Only ever runs in WebView2 / WebKit, so no legacy transpiling is needed.
    target: "chrome114",
    sourcemap: false,
  },
});
