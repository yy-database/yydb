import fs from 'node:fs';
import path from 'node:path';
import type { Cli, ParsedOptions } from '@vmz/commander';
import { formatDiagnostic } from '@vmz/diagnostic';

import { loadNative } from '../native/binding.js';
import type { DoctorIssue, DoctorReport } from '../api/database.js';
import { requiredPositional } from './options.js';

/**
 * Inline CLI catalog — commander's `.locales()` resolution is not exposed
 * outside the CLI builder, so we ship a static copy here. Only one locale is
 * loaded at runtime; future locale awareness should resolve the chosen locale
 * from commander internals once they surface it.
 */
const CLI_CATALOG: Record<string, string> = JSON.parse(
    fs.readFileSync(path.resolve(import.meta.dirname, '../../locales/en-US/cli.json'), 'utf8'),
);

const DIAGNOSTIC_STYLE = 'compact' as const;
const DIAGNOSTIC_LOCALE = 'en-US';

export function registerVerifyCommand(cli: Cli): void {
    cli.command('verify', 'cli.cmd.verify').action((options) => cmdVerify(options));
}

/**
 * Run read-only consistency probes against a YYDB database file.
 *
 * Exit codes:
 *   0  — clean (no issues)
 *   1  — doctor found at least one error (database may be corrupt)
 *   13 — only warnings present (readable but should be inspected)
 */
export function cmdVerify(options: ParsedOptions): number {
    const dbPath = requiredPositional(options, 'verify');

    let report: DoctorReport;
    try {
        const native = loadNative();
        const db = native.Database.open(dbPath);
        report = JSON.parse(db.doctor()) as DoctorReport;
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        console.error(formatDiagnostic({
            path: dbPath,
            severity: 'error',
            code: 'cli.err.open_failed',
            args: { path: dbPath, reason: message },
        }, {
            locale: DIAGNOSTIC_LOCALE,
            catalog: CLI_CATALOG,
            style: DIAGNOSTIC_STYLE,
        }));
        return 1;
    }

    for (const issue of report.issues) {
        const severity: 'error' | 'warning' = issue.severity === 'error' ? 'error' : 'warning';
        console.error(formatDiagnostic({
            path: dbPath,
            severity,
            code: issue.code,
            message: issue.message,
        }, {
            locale: DIAGNOSTIC_LOCALE,
            catalog: CLI_CATALOG,
            style: DIAGNOSTIC_STYLE,
        }));
    }

    const errCount = report.issues.filter((i) => i.severity === 'error').length;
    const warnCount = report.issues.length - errCount;

    if (errCount > 0) {
        console.log(`exit=1 errors=${errCount} warnings=${warnCount}`);
        return 1;
    }
    if (warnCount > 0) {
        console.log(`exit=13 errors=0 warnings=${warnCount}`);
        return 13;
    }
    console.log(`exit=0 clean last_commit=${report.last_commit_sequence}`);
    return 0;
}
