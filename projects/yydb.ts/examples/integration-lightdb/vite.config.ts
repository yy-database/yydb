import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const root = dirname(fileURLToPath(import.meta.url));
const wasmRoot = join(root, "..", "..", "platforms", "yydb-unknown-wasm32", "lib");

export default defineConfig({
    root: join(root, "browser"),
    build: {
        outDir: join(root, "dist"),
        emptyOutDir: true,
        target: "esnext",
        rollupOptions: {
            input: join(root, "browser", "index.html"),
        },
    },
    publicDir: wasmRoot,
    server: {
        fs: {
            allow: [root, wasmRoot],
        },
    },
});
