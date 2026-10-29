import path from "node:path";
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  resolve: { alias: { "~": path.resolve(__dirname, "src") } },
  server: {
    host: true,
    port: 5173,
    proxy: { "/api": "http://localhost:3000" },
  },
});
