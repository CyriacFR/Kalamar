import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(({ mode }) => {
  // `vite --mode mock` (or VITE_KALAMAR_MOCK=1): the page runs in a plain browser on made-up data (src/dev/mock/), to
  // look at every screen without the Tauri back end. The production build never takes this branch, so the mock is not
  // in it.
  const mock = mode === "mock" || process.env.VITE_KALAMAR_MOCK === "1";
  const dir = (p: string) => new URL(p, import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
  return {
    plugins: [react()],
    ...(mock
      ? {
          resolve: {
            alias: {
              "@tauri-apps/api/core": dir("./src/dev/mock/core.ts"),
              "@tauri-apps/api/event": dir("./src/dev/mock/event.ts"),
              "@tauri-apps/plugin-dialog": dir("./src/dev/mock/dialog.ts"),
            },
          },
          define: { "import.meta.env.VITE_KALAMAR_MOCK": JSON.stringify("1") },
        }
      : {}),

    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    //
    // 1. prevent Vite from obscuring rust errors
    clearScreen: false,
    // 2. tauri expects a fixed port, fail if that port is not available
    server: {
      port: 1420,
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 1421,
          }
        : undefined,
      watch: {
        // 3. tell Vite to ignore watching `src-tauri`
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
