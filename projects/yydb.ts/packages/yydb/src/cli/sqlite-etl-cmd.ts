import type { Cli, ParsedOptions } from '@vmz/commander';

import { requiredPositional, str } from './options.js';
import { exportToSqlite, importFromSqlite } from './sqlite-etl.js';

export function registerImportCommand(cli: Cli): void {
    cli.command('import', 'cli.cmd.import')
        .option('--into <path>', 'cli.opt.import-into')
        .option('--table <name>', 'cli.opt.table')
        .option('--prefix <text>', 'cli.opt.prefix')
        .action((options) => cmdImport(options));
}

export function registerExportCommand(cli: Cli): void {
    cli.command('export', 'cli.cmd.export')
        .option('--sqlite <path>', 'cli.opt.export-sqlite')
        .option('--table <name>', 'cli.opt.table')
        .option('--prefix <text>', 'cli.opt.prefix')
        .action((options) => cmdExport(options));
}

/** Import rows from a SQLite `(key TEXT, value BLOB)` table into YYDB. */
export async function cmdImport(options: ParsedOptions): Promise<number> {
    const source = requiredPositional(options, 'import');
    const into = str(options, 'into');
    if (!into) {
        throw new Error('import requires --into <database path>');
    }
    const result = await importFromSqlite(source, into, {
        table: str(options, 'table'),
        prefix: str(options, 'prefix'),
    });
    console.log(`imported ${result.rows} rows from ${result.table} into ${into}`);
    return 0;
}

/** Export YYDB records into a SQLite `(key TEXT, value BLOB)` table. */
export async function cmdExport(options: ParsedOptions): Promise<number> {
    const database = requiredPositional(options, 'export');
    const target = str(options, 'sqlite');
    if (!target) {
        throw new Error('export requires --sqlite <file path>');
    }
    const result = await exportToSqlite(database, target, {
        table: str(options, 'table'),
        prefix: str(options, 'prefix'),
    });
    console.log(`exported ${result.rows} rows into ${target} table ${result.table}`);
    return 0;
}
