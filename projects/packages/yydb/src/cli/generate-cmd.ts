import fs from 'node:fs';
import path from 'node:path';

import type { Cli, ParsedOptions } from '@vmz/commander';

import { generateTypeScript } from '../generator/index.js';
import { requiredPositional, str } from './options.js';

export function registerGenerateCommand(cli: Cli): void {
    cli.command('generate', 'cli.cmd.generate')
        .option('-o, --output <path>', 'cli.opt.output')
        .option('--schema-version <n>', 'cli.opt.schema-version')
        .action((options) => cmdGenerate(options));
}

export function cmdGenerate(options: ParsedOptions): number {
    const input = requiredPositional(options, 'generate');
    const output = str(options, 'output');
    if (!output) {
        throw new Error('generate requires --output <path>');
    }
    const schemaVersion = Number.parseInt(str(options, 'schema-version') ?? '1', 10);
    const source = fs.readFileSync(input, 'utf8');
    const ts = generateTypeScript(source, schemaVersion);
    const outPath = path.resolve(output);
    fs.mkdirSync(path.dirname(outPath), { recursive: true });
    fs.writeFileSync(outPath, ts);
    console.log(`wrote ${outPath}`);
    return 0;
}
