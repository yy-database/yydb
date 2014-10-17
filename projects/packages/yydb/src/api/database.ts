import fs from 'node:fs';
import path from 'node:path';
import {
    emitSource,
    type EmittableItem,
} from '@voml/vos';
import {
    defineMicro as buildMicro,
    microFingerprint,
    udfWireKind,
    type DefinedMicro,
    type MicroHandle,
    MicroSessionRegistry,
} from '../shared/micro.js';
import { loadNative } from '../native/binding.js';
import type { UdfScalarDescriptor } from '../shared/udf-types.js';

/** Published schema metadata. */
export interface SchemaVersion {
    version: number;
    document: string;
}

/** Options for opening a local YYDB file. */
export interface OpenOptions {
    /** Create parent directories when missing (default: true). */
    mkdir?: boolean;
}

/**
 * Node-facing YYDB local database handle.
 *
 * Directly owns a native `yydb::Connection` via the Node-API binding. No
 * child process, no loopback TCP, no wire protocol in the hot path.
 */
export class Database {
    readonly path: string;
    private readonly native: ReturnType<ReturnType<typeof loadNative>['Database']['open']>;
    private readonly microRegistry: MicroSessionRegistry;

    private constructor(
        dbPath: string,
        native: ReturnType<ReturnType<typeof loadNative>['Database']['open']>,
    ) {
        this.path = dbPath;
        this.native = native;
        this.microRegistry = new MicroSessionRegistry();
    }

    /** Open (create when missing) the database at `dbPath`. */
    static async open(dbPath: string, options: OpenOptions = {}): Promise<Database> {
        const resolved = path.resolve(dbPath);
        if (options.mkdir !== false) {
            fs.mkdirSync(path.dirname(resolved), { recursive: true });
        }

        const native = loadNative();
        const db = native.Database.open(resolved);
        return new Database(resolved, db);
    }

    /** Engine version string (same as `yydb --version`). */
    serverVersion(): string {
        return loadNative().version();
    }

    /** Basic diagnostics: path, schema, record count, journal mode. */
    info(): string {
        const lines = [
            `path=${this.native.path() ?? '<memory>'}`,
            `schema.version=${this.native.schemaVersion() ?? ''}`,
            `records=${this.native.recordCount()}`,
            `journal_mode=${this.native.journalMode()}`,
        ];
        return lines.join('\n') + '\n';
    }

    /** Published schema metadata, or `null` before the first schema. */
    async getSchema(): Promise<{ version: number; document: string } | null> {
        const version = this.native.schemaVersion();
        const document = this.native.schemaDocument();
        if (version === null || document === null) {
            return null;
        }
        return { version, document };
    }

    /**
     * Validate and publish a VOS schema document.
     *
     * Accepts either a raw `.vos` source string, one `@voml/vos` descriptor
     * (`v.table()`, `v.class()`) or a list of descriptors. When descriptors
     * are supplied, they are rendered to source via `emitSource` first.
     *
     * ```ts
     * // Raw source (unchanged behaviour):
     * await db.ensureSchema('namespace demo { table User { @@id: uuid } }');
     *
     * // Zod-like builder — the Vos schema-ecosystem sweet spot:
     * const User = v.table('User', { id: v.uuid().primary(), name: v.utf8() });
     * await db.ensureSchema(User);
     *
     * // Multiple declarations in one shot:
     * await db.ensureSchema(TableA, TableB, ClassC);
     * ```
     */
    async ensureSchema(...input: [string] | [string, ...EmittableItem[]] | EmittableItem[]): Promise<void> {
        let document: string;
        if (typeof input[0] === 'string') {
            // Raw source string, optionally followed by descriptors we just ignore.
            document = input[0];
        } else {
            // One or more Vos descriptor items → render to source.
            document = emitSource(input as EmittableItem[]);
        }
        this.native.ensureSchema(document);
    }

    /** Read one record, or `null` when the key is absent. */
    async get(key: string): Promise<Uint8Array | null> {
        return this.native.get(key);
    }

    /** Insert or replace one record. */
    async put(key: string, value: Uint8Array | string): Promise<void> {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        this.native.put(key, bytes);
    }

    /** Fold the WAL into the main file and truncate the sidecar. */
    async checkpoint(): Promise<void> {
        this.native.checkpoint();
    }

    /** Build a typed session-local micro definition (without registering). */
    defineMicro<TArgs extends readonly UdfScalarDescriptor[], TReturn extends UdfScalarDescriptor>(
        definition: Parameters<typeof buildMicro<TArgs, TReturn>>[0],
    ): DefinedMicro<TArgs, TReturn> {
        return buildMicro(definition);
    }

    /**
     * Install a session-local host micro into both the local registry and the
     * native engine. Host micros never persist across processes.
     */
    async registerMicro<TArgs extends readonly UdfScalarDescriptor[], TReturn extends UdfScalarDescriptor>(
        definition: DefinedMicro<TArgs, TReturn>,
    ): Promise<MicroHandle> {
        const handle = this.microRegistry.register(definition);
        const argTypes = definition.args.map((d) => udfWireKind(d));
        const returnType = udfWireKind(definition.returns);
        this.native.registerMicro(
            definition.name,
            definition.version,
            argTypes,
            returnType,
            handle.hostId,
            handle.functionId,
            handle.implementationVersion,
            microFingerprint(definition.name, definition.version),
        );
        return handle;
    }

    /** Invoke a registered scalar UDF on the native engine. */
    async callScalar(
        name: string,
        version: number,
        args: readonly (string | number | boolean | null)[],
    ): Promise<string | number | boolean | null> {
        const argsJson = JSON.stringify(args);
        const resultJson = this.native.callScalar(name, version, argsJson);
        return JSON.parse(resultJson);
    }

    /** Release the native handle (best-effort; the engine also drops on GC). */
    async close(): Promise<void> {
        // Native connections drop cleanly when the handle is garbage-collected.
        // No explicit close() is exposed from yydb::Connection.
    }
}
