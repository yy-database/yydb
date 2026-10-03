import type { SchemaField, SchemaTable, ScalarType, TsSchemaIr } from "./ir.js";

export class VosParseError extends Error {
    constructor(message: string) {
        super(message);
        this.name = "VosParseError";
    }
}

/** Interim VOS subset parser for `yydb generate` (MVP). */
export function parseVosSubset(source: string, schemaVersion: number): TsSchemaIr {
    if (source.includes("\0")) {
        throw new VosParseError("VOS document must not contain NUL bytes");
    }

    let normalized = source.startsWith("\uFEFF") ? source.slice(1) : source;
    normalized = normalized.replace(/\r\n/g, "\n").replace(/\r/g, "\n");

    const lexer = new Lexer(normalized);
    const tables: SchemaTable[] = [];
    while (!lexer.eof()) {
        lexer.skipWsAndComments();
        if (lexer.eof()) {
            break;
        }
        tables.push(parseTable(lexer));
    }
    if (tables.length === 0) {
        throw new VosParseError("expected at least one `table` declaration");
    }
    return {
        vosSource: normalized,
        schemaVersion,
        tables,
    };
}

function parseTable(lexer: Lexer): SchemaTable {
    lexer.expectIdent("table");
    const name = lexer.expectAnyIdent();
    lexer.expectPunct("{");
    const fields: SchemaField[] = [];
    while (true) {
        lexer.skipWsAndComments();
        if (lexer.eatPunct("}")) {
            break;
        }
        if (lexer.eatPunct("@")) {
            lexer.expectPunct("@");
            const attr = lexer.expectAnyIdent();
            if (attr !== "id") {
                throw new VosParseError(
                    `unsupported table attribute \`@@${attr}\` (MVP supports \`@@id\` only)`,
                );
            }
            lexer.expectPunct(":");
            const ty = parseScalar(lexer);
            fields.push({ name: "id", ty, isId: true });
        } else {
            const fieldName = lexer.expectAnyIdent();
            lexer.expectPunct(":");
            const ty = parseScalar(lexer);
            fields.push({ name: fieldName, ty, isId: false });
        }
        lexer.skipWsAndComments();
        lexer.eatPunct(",");
    }
    if (!fields.some((field) => field.isId)) {
        throw new VosParseError(`table \`${name}\` requires \`@@id: <type>\``);
    }
    return { name, fields };
}

function parseScalar(lexer: Lexer): ScalarType {
    const name = lexer.expectAnyIdent();
    switch (name) {
        case "uuid":
            return "uuid";
        case "utf8":
            return "utf8";
        case "bool":
            return "bool";
        case "i64":
            return "i64";
        case "f64":
            return "f64";
        default:
            throw new VosParseError(
                `unsupported scalar type \`${name}\` (MVP: uuid, utf8, bool, i64, f64)`,
            );
    }
}

class Lexer {
    private i = 0;

    constructor(private readonly src: string) {}

    eof(): boolean {
        return this.i >= this.src.length;
    }

    private peek(): string | undefined {
        return this.src[this.i];
    }

    private bump(): string | undefined {
        const ch = this.peek();
        if (ch === undefined) {
            return undefined;
        }
        this.i += 1;
        return ch;
    }

    skipWsAndComments(): void {
        while (true) {
            while (this.peek() !== undefined && /\s/.test(this.peek()!)) {
                this.bump();
            }
            if (this.src.startsWith("//", this.i)) {
                while (this.peek() !== undefined && this.peek() !== "\n") {
                    this.bump();
                }
                continue;
            }
            break;
        }
    }

    eatPunct(expected: string): boolean {
        this.skipWsAndComments();
        if (this.peek() === expected) {
            this.bump();
            return true;
        }
        return false;
    }

    expectPunct(expected: string): void {
        if (!this.eatPunct(expected)) {
            throw new VosParseError(`expected \`${expected}\` at byte ${this.i}`);
        }
    }

    expectIdent(expected: string): void {
        const got = this.expectAnyIdent();
        if (got !== expected) {
            throw new VosParseError(`expected \`${expected}\`, found \`${got}\``);
        }
    }

    expectAnyIdent(): string {
        this.skipWsAndComments();
        const start = this.i;
        const first = this.peek();
        if (first === undefined || !/[A-Za-z_]/.test(first)) {
            throw new VosParseError(`expected identifier at byte ${this.i}`);
        }
        this.bump();
        while (this.peek() !== undefined && /[A-Za-z0-9_]/.test(this.peek()!)) {
            this.bump();
        }
        return this.src.slice(start, this.i);
    }
}
