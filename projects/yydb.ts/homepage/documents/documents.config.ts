/**
 * YYDB integrated documents — mounted at `/d/`.
 */
export default {
    defaultLocale: "zh-hans",
    locales: {
        "zh-hans": { label: "简体中文" },
    },
    collections: {
        default: {
            source: ".",
            mount: "/d",
        },
    },
};
