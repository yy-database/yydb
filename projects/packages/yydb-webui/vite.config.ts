import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
    plugins: [vue()],
    resolve: {
        alias: {
            "@yydb/yydb-client": fileURLToPath(
                new URL("../yydb-client/src/index.ts", import.meta.url),
            ),
        },
    },
    server: {
        port: 5177,
    },
});
