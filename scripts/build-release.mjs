/**
 * Build the artifacts every publishing path needs.
 *
 * `check-typescript.yml` and `release-npm.yml` used to keep separate build
 * lists and the release list drifted: it skipped the WASM platform package
 * that `@yydb/light-db` imports, so `tsc` failed only at publish time while
 * CI stayed green. Both workflows call this file so the order has one owner.
 */
import { spawnSync } from 'node:child_process';

const steps = [
    ['pnpm', ['run', 'build:wasm']],
    ['pnpm', ['--filter', '@yydb/yydb-unknown-wasm32', 'build']],
    ['pnpm', ['--filter', '@yydb/yydb-client', 'build']],
    ['pnpm', ['--filter', '@yydb/light-db', 'build']],
    ['pnpm', ['--filter', '@yydb/yydb', 'build']],
];

for (const [command, args] of steps) {
    console.log(`$ ${command} ${args.join(' ')}`);
    const result = spawnSync(command, args, { stdio: 'inherit', shell: true, env: process.env });
    if (result.status !== 0) {
        process.exit(result.status ?? 1);
    }
}
