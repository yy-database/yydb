import { defineConfig } from "@vmz/plugin";

export default defineConfig({
    delivery: {
        default: "static",
        profiles: {
            static: { host: "browser", assembly: "web-static", name: "cdn" },
        },
    },
});
