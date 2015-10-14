import fs from 'node:fs';
import path from 'node:path';
import type { Cli, ParsedOptions } from '@vmz/commander';

import { Database } from '../api/database.js';
import { requiredPositional, str } from './options.js';

export function registerRestoreCommand(cli: Cli): void {
    cli.command('restore', 'cli.cmd.restore')
        .option('--target <path>', 'cli.opt.restore-target')
        .action((options) => cmdRestore(options));
}

/**
 * Verify a backup and optionally copy it to a destination.
 *
 * The backup is opened first so every integrity probe runs before anything
 * reaches the destination. `.yydx` backups hold a sibling object tree, so
 * copying them file-by-file is refused rather than half-done.
 */
export async function cmdRestore(options: ParsedOptions): Promise<number> {
    const backupPath = requiredPositional(options, 'restore');
    const target = str(options, 'target');

    const probe = await Database.restore(backupPath);
    let summary = probe.info().trim();
    await probe.close();

    if (target) {
        if (backupPath.endsWith('.yydx')) {
            throw new Error('restoring a .yydx layout to a target is not supported: copy the object tree yourself');
        }
        const resolved = path.resolve(target);
        fs.mkdirSync(path.dirname(resolved), { recursive: true });
        fs.copyFileSync(path.resolve(backupPath), resolved);

        const reopened = await Database.open(resolved);
        summary = reopened.info().trim();
        await reopened.close();
        console.log(`restored ${backupPath} -> ${resolved}`);
    }
    console.log(summary);
    return 0;
}
