import type { ParsedOptions } from '@vmz/commander';

export function str(options: ParsedOptions, key: string): string | undefined {
    const value = options[key];
    return typeof value === 'string' && value.length > 0 ? value : undefined;
}

export function flag(options: ParsedOptions, key: string): boolean {
    return options[key] === true;
}

export function requiredPositional(options: ParsedOptions, label: string): string {
    const value = options._[0];
    if (!value) {
        throw new Error(`${label} requires a path argument`);
    }
    return value;
}
