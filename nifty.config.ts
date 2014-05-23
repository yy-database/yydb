// Nifty project configuration for yydb (hybrid cargo + pnpm).
import { defineConfig } from '@doki-land/nifty';

export default defineConfig({
    format: {
        preset: 'nifty',
        style: {
            indentStyle: 'space',
            indentWidth: 4,
            lineWidth: 144,
            quoteStyle: 'single',
        },
        includes: ['package.json', 'nifty.config.ts', 'scripts/**', 'projects/packages/**'],
        excludes: ['**/node_modules/**', '**/dist/**', '**/target/**', '**/.vite/**', '**/pnpm-lock.yaml', '**/*.md', '**/*.css', '**/*.vue'],
    },
    trust: {
        npm: {
            repo: 'yy-database/yydb',
            file: 'release-npm.yml',
            environment: 'NPM_PUBLISH',
        },
    },
    publish: {
        packages: [
            '@yydb/yydb-win32-x64',
            '@yydb/yydb-linux-x64',
            '@yydb/yydb-darwin-x64',
            '@yydb/yydb-darwin-arm64',
            '@yydb/yydb-client',
            '@yydb/yydb',
        ],
    },
});
