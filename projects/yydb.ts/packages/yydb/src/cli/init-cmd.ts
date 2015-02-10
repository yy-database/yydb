import fs from 'node:fs';
import type { Cli, ParsedOptions } from '@vmz/commander';

import { loadNative } from '../native/binding.js';
import { requiredPositional, str } from './options.js';

export function registerInitCommand(cli: Cli): void {
    cli.command('init', 'cli.cmd.init')
        .option('--schema-file <path>', 'cli.opt.schema-file')
        .action((options) => cmdInit(options));
}

export function cmdInit(options: ParsedOptions): number {
    const dbPath = requiredPositional(options, 'init');
    const schemaFile = str(options, 'schema-file');
    const database = loadNative().Database.open(dbPath);
    if (schemaFile) {
        database.ensureSchema(fs.readFileSync(schemaFile, 'utf8'));
    }
    console.log(`initialized ${dbPath}`);
    return 0;
}
