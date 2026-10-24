/**
 * Browser database authoring-surface types.
 *
 * These are the stable contract for `@yydb/light-db`:
 * - Binary values are always `Uint8Array` — never `Buffer`.
 * - No local file paths, file handles, or Node-exclusive types leak through.
 * - Aligned with the Node-side `@yydb/yydb` method names and return semantics.
 */

/** Schema version and document (aligned with `@yydb/yydb`). */
export interface SchemaVersion {
    version: number;
    document: string;
}

/** One row from {@link Database.scan}. */
export interface ScanRow {
    key: string;
    value: Uint8Array;
}

/** Options for {@link Database.scan}. */
export interface ScanOptions {
    /** Include this prefix in every returned key; default `""` returns everything. */
    prefix?: string;
    /** Exclusive continuation cursor from a previous scan's last row; default `null` starts fresh. */
    after?: string | null;
    /** Max rows returned per call; default `1000`. */
    limit?: number;
}

/** One change in {@link Database.batch}. Use `null` to delete. */
export interface BatchChange {
    key: string;
    value: Uint8Array | string | null;
}

/** Options for {@link Database.migrateSchema}. */
export interface MigrateOptions {
    /** Rename entries — `fromType`/`toType` updates a type name; `type`/`fromField`/`toField` updates one field. */
    renames?: ReadonlyArray<
        | { fromType: string; toType: string }
        | { type: string; fromField: string; toField: string }
    >;
}

/** Options for opening the WASM-backed {@link Database}. */
export interface OpenOptions {
    /** Use an in-memory session (test only; default is OPFS persistent). */
    memory?: boolean;
}

// ---------------------------------------------------------------------------
// Doctor report (Rust-side shape — mirrored in WASM)
// ---------------------------------------------------------------------------

namespace yydbDoctor {
    /** Per-namespace summary from the doctor. */
    export interface NamespaceStats {
        name: string;
        record_count: number;
        object_count: number;
        total_bytes: number;
    }

    /** One named diagnostic issued by the doctor. */
    export interface Issue {
        level: 'info' | 'warn' | 'error';
        code: string;
        message: string;
    }
}

/** Read-only consistency probe report. */
export interface DoctorReport {
    file_path?: string | null;
    last_commit_sequence: number;
    last_checkpoint_sequence: number;
    checkpoint_lag_records: number;
    wal_bytes_uncheckpointed: number;
    orphan_object_count: number;
    namespaces: yydbDoctor.NamespaceStats[];
    issues: yydbDoctor.Issue[];
}

// ---------------------------------------------------------------------------
// Prepared statement — typed mirror of Node-side `@yydb/yydb` author surface
// ---------------------------------------------------------------------------

/** VOS parameter slot type. */
export type SlotType = 'any' | 'bool' | 'i64' | 'text' | 'null';

// ---------------------------------------------------------------------------
// Exact scalar wire forms — mirrored from the Rust `yydb-types` scalar set
// ---------------------------------------------------------------------------

/** Tagged JSON form of an exact `date` value (days since the Unix epoch). */
export interface DateScalar {
    readonly $yydb: 'date';
    readonly days: number;
}

/** Tagged JSON form of an exact `time` value (nanoseconds since midnight, no zone). */
export interface TimeScalar {
    readonly $yydb: 'time';
    readonly nanos: number;
}

/** Tagged JSON form of an exact UTC `timestamp` (microseconds as a decimal string). */
export interface TimestampScalar {
    readonly $yydb: 'timestamp';
    readonly utcMicros: string;
}

/** Tagged JSON form of an exact fixed-point `decimal` value. */
export interface DecimalScalar {
    readonly $yydb: 'decimal';
    readonly precision: number;
    readonly scale: number;
    readonly unscaled: string;
}

/** Every exact scalar crossing the WASM JSON boundary uses this tagged form. */
export type ScalarValue = DateScalar | TimeScalar | TimestampScalar | DecimalScalar;

/** Declared execution type of a catalog field. */
export type SchemaFieldType =
    | { readonly kind: 'null' | 'bool' | 'i64' | 'text' | 'bytes' | 'file' | 'date' }
    | { readonly kind: 'time'; readonly unit: 'nanosecond'; readonly timezone: 'none' }
    | { readonly kind: 'timestamp'; readonly unit: 'microsecond'; readonly timezone: 'utc' }
    | { readonly kind: 'decimal'; readonly precision: number; readonly scale: number }
    | { readonly kind: 'vector'; readonly dimension: number; readonly metric: 'cosine' | 'euclidean' | 'dot' }
    | { readonly kind: 'record'; readonly schemaId: number; readonly fieldCount: number };

/** One uniqueness rule published by the catalog. */
export interface UniqueConstraintDescriptor {
    readonly name: string;
    readonly columns: string[];
    /** `true` means NULLs never collide (SQL default); `false` means one shared NULL. */
    readonly nullsDistinct: boolean;
}

/** One field in an introspected table. */
export interface SchemaFieldDescriptor {
    readonly name: string;
    readonly fieldId: number;
    readonly virtualField: number;
    readonly type: SchemaFieldType;
}

/** One introspected table with declared field types and uniqueness rules. */
export interface SchemaTableDescriptor {
    readonly name: string;
    readonly schemaId: number;
    readonly fields: SchemaFieldDescriptor[];
    readonly uniqueConstraints: UniqueConstraintDescriptor[];
}

/** Payload returned by `introspectSchema`. */
export interface SchemaIntrospection {
    readonly ok: boolean;
    readonly schemaFingerprint: string;
    readonly tables: SchemaTableDescriptor[];
    readonly error: unknown;
}

/** One named parameter descriptor on a prepared statement. */
export interface ParameterDescriptor {
    /** Canonical parameter name (no leading `$`). */
    name: string;
    /** Positional slot index within the bound-value slice (0-based). */
    index: number;
    /** Declared slot type; narrowed from surrounding context when possible. */
    slotTy: SlotType;
    /** Number of times this parameter appears in the source. */
    usageCount: number;
}

/** Whether a prepared statement is a read pipeline or a write program. */
export type QueryKind = 'Read' | 'Write';

/** Metadata about a prepared statement — useful for logging and cache keys. */
export interface StatementInfo {
    statement_id: string;
    query_kind: QueryKind;
    parameters: ParameterDescriptor[];
    read_set: string[];
    write_set: string[];
    source_fingerprint: string;
    source: string;
}

/**
 * Light database contract — aligned with the Node-side `@yydb/yydb`.
 * Only the WASM-backed {@link Database} implements this in production;
 * the formerly pure-TS backend has been removed.
 */
export interface LightDatabase {
    /** Database name (OPFS path / namespace). */
    readonly name: string;

    /** Engine version string. */
    serverVersion(): string;

    /** Basic diagnostics — session info from the engine. */
    info(): string;

    /**
     * Read-only consistency probe — same semantics as the Node-side
     * `@yydb/yydb` doctor. Returns a report with checkpoint lag,
     * orphan counts, namespace stats and any named issues.
     */
    doctor(): Promise<DoctorReport>;

    /** Published schema metadata, or `null` before the first schema. */
    getSchema(): Promise<SchemaVersion | null>;

    /**
     * Validate and publish a VOS schema document. Accepts raw `.vos`
     * source or `@voml/vos` descriptors (e.g. `v.table()`, `v.class()`).
     */
    ensureSchema(...input: [string] | object[]): Promise<void>;

    /** Read one record, or `null`. */
    get(key: string): Promise<Uint8Array | null>;

    /** Insert or replace one record. */
    put(key: string, value: Uint8Array | string): Promise<void>;

    /** Delete one record. Silently succeeds when absent. */
    delete(key: string): Promise<void>;

    /** Fold the WAL into the main storage. */
    checkpoint(): Promise<void>;

    /** Begin an explicit transaction. */
    begin(): Promise<void>;

    /** Commit the current transaction. */
    commit(): Promise<void>;

    /** Roll back the current transaction. */
    rollback(): Promise<void>;

    /** Execute a VOS read pipeline. */
    query<T = Record<string, unknown>>(source: string): Promise<T[]>;

    /** Execute a VOS write program. */
    execute(source: string): Promise<void>;

    /** Close the session. Idempotent. */
    close(): Promise<void>;
}
