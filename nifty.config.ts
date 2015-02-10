// Nifty project configuration for yydb (hybrid cargo + pnpm).
import { defineConfig } from '@doki-land/nifty';

export default defineConfig({
    authorMap: 'documentation/maintenance/author-github.json',
    changelog: {
        repo: 'yy-database/yydb',
        releasesDir: 'documentation/maintenance/releases',
    },
    format: {
        preset: 'nifty',
        style: {
            indentStyle: 'space',
            indentWidth: 4,
            lineWidth: 144,
            quoteStyle: 'single',
        },
        includes: ['package.json', 'nifty.config.ts', 'scripts/**', 'projects/yydb.ts/**'],
        excludes: [
            '**/node_modules/**',
            '**/dist/**',
            '**/target/**',
            '**/.vite/**',
            '**/pnpm-lock.yaml',
            '**/*.md',
            '**/*.css',
            '**/*.vue',
            'projects/yydb.ts/platforms/yydb-unknown-wasm32/lib/**',
        ],
    },
    trust: {
        npm: {
            repo: 'yy-database/yydb',
            file: 'release-npm.yml',
            environment: 'NPM_PUBLISH',
        },
    },
});
