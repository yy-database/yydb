import { defineConfig } from "@vmz/plugin";
import markdownIt from "@vmz/plugin-markdown-it";
import shiki from "@vmz/plugin-shiki";

export default defineConfig({
    plugins: [markdownIt(), shiki()],
    engines: { code: "shiki", markdown: "markdown-it" },
    delivery: {
        default: "static",
        profiles: {
            static: { host: "browser", assembly: "web-static", name: "cdn" },
        },
    },
});
