import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const root = dirname(fileURLToPath(import.meta.url));
// tests/browser → packages/light-db → packages → yydb.ts root (projects/yydb.ts)
const yydbTsRoot = join(root, "..", "..", "..");
const wasmRoot = join(yydbTsRoot, "platforms", "yydb-unknown-wasm32", "lib");
const packageRoot = join(root, "..", "..");

export default defineConfig({
    root,
    resolve: {
        // The package cannot self-reference `@yydb/light-db` (no `exports` field),
        // so the browser bundle points at the source entry directly.
        alias: { "@yydb/light-db": join(packageRoot, "src", "index.ts") },
    },
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
