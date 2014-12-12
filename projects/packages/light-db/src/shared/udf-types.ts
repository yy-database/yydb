/**
 * Host micro (UDF) boundary types — browser WASM product subset.
 *
 * Descriptors come straight from `@voml/vos`: `v.i64()`, `v.bool()`, `v.utf8()`.
 * `@yydb/light-db` keeps no scalar descriptor algebra of its own — it simply
 * re-exports the VOS descriptors the Phase-1 UDF boundary accepts.
 *
 * `null` is not a descriptor; it is a value carried by the wire subset.
 * This module is types-only and can be stripped at compile time.
 */

import type { Infer, TypeDescriptor } from '@voml/vos';

/** Phase-1 scalar kinds on the YY wire micro path, aligned with `yydb-udf`. */
export type UdfScalarKind = 'i64' | 'bool' | 'text' | 'null';

/**
 * VOS descriptors accepted at the Phase-1 host micro boundary.
 *
 * Non-scalar descriptors (`v.optional(…)`, `v.list(…)`, `v.utf16()`, …) are
 * rejected before they reach the wire.
 */
export type UdfScalarDescriptor =
    | TypeDescriptor<'i64', bigint>
    | TypeDescriptor<'bool', boolean>
    | TypeDescriptor<'utf8', string>;

/** Maps a descriptor tuple to positional boundary value types. */
export type InferUdfArgs<T extends readonly UdfScalarDescriptor[]> = {
    [K in keyof T]: Infer<T[K]>;
};

/** Maps a return descriptor to its boundary value type. */
export type InferUdfReturn<T extends UdfScalarDescriptor> = Infer<T>;

/** Session micro effect surface (Phase 1: pure only). */
export type MicroEffect = 'pure';

/** Invocation mode — scalar for Phase 1, batch reserved for future. */
export type MicroInvocationMode = 'scalar' | 'batch';
