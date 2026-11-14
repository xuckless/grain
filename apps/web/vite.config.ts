import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { createReadStream, existsSync } from "node:fs";
import { basename, resolve } from "node:path";

// Cross-origin isolation (SharedArrayBuffer, threaded onnxruntime-web), AGENTS.md §9.
const isolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

/** Serves models/weights/* at /models/* in dev and preview (weights are never bundled). */
function modelWeights(): Plugin {
  const dir = resolve(import.meta.dirname, "../../models/weights");
  // Returns nothing: a function returned from a configure hook would run as a post hook.
  const serve = (server: { middlewares: { use: (path: string, fn: (req: any, res: any, next: () => void) => void) => unknown } }): void => {
    server.middlewares.use("/models", (req, res, next) => {
      const file = resolve(dir, basename(String(req.url ?? "").split("?")[0]));
      if (!existsSync(file)) return next();
      res.setHeader("Cross-Origin-Resource-Policy", "same-origin");
      createReadStream(file).pipe(res);
    });
  };
  return { name: "model-weights", configureServer: serve, configurePreviewServer: serve };
}

export default defineConfig({
  plugins: [svelte(), modelWeights()],
  server: { headers: isolation },
  preview: { headers: isolation },
  optimizeDeps: { exclude: ["onnxruntime-web", "engine-wasm"] },
  worker: { format: "es" },
});
