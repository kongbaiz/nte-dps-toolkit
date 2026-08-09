import path from "node:path";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, searchForWorkspaceRoot } from "vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      "@res": path.resolve(__dirname, "../res"),
    },
  },
  build: {
    rollupOptions: {
      input: {
        "main-dps": path.resolve(__dirname, "main-dps.html"),
        console: path.resolve(__dirname, "console.html"),
        hud: path.resolve(__dirname, "hud.html"),
        notification: path.resolve(__dirname, "notification.html"),
        "combat-details": path.resolve(__dirname, "combat-details.html"),
        "abyss-values": path.resolve(__dirname, "abyss-values.html"),
      },
    },
  },
  server: {
    fs: {
      allow: [
        searchForWorkspaceRoot(process.cwd()),
        path.resolve(__dirname, "../res"),
      ],
    },
  },
});
