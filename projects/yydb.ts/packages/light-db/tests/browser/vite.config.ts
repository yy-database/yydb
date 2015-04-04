import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const root = dirname(fileURLToPath(import.meta.url));
// tests/browser → packages/light-db → packages → yydb.ts root (projects/yydb.ts)
const yydbTsRoot = join(root, "..", "..", "..");
const wasmRoot = join(yydbTsRoot, "platforms", "yydb-unknown-wasm32", "lib");

export default defineConfig({
    root,
    build: {
        outDir: join(root, "dist"),
        emptyOutDir: true,
        target: "esnext",
        rollupOptions: {
            input: join(root, "index.html"),
        },
    },
    publicDir: wasmRoot,
    server: {
        fs: {
            allow: [root, wasmRoot],
        },
    },
});
