import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, import.meta.dirname, "STELLARDB_");
  const apiTarget =
    env.STELLARDB_DEV_SERVER_URL || "http://127.0.0.1:3000";

  return {
    base: "/_station/",
    plugins: [react()],
    define: {
      __STELLARDB_DEV_PROXY_TARGET__: JSON.stringify(apiTarget),
    },
    resolve: {
      alias: {
        "@stellardb/client": path.resolve(
          import.meta.dirname,
          "../packages/client/src",
        ),
        "@": path.resolve(import.meta.dirname, "./src"),
      },
    },
    server: {
      proxy: {
        "/v1": apiTarget,
        "/stats": apiTarget,
        "/databases": apiTarget,
        "/auth": apiTarget,
      },
    },
  };
});
