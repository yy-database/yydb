/**
 * Host micro (UDF) boundary types.
 *
 * Descriptors come straight from `@voml/vos`: `v.i64()`, `v.bool()`, `v.utf8()`.
 * YYDB keeps no scalar descriptor algebra of its own — only the VOS descriptors
 * the Phase-1 UDF boundary accepts.
 *
 * `null` is not a type here. It is a value carried by the wire subset, so it
 * gets no descriptor.
 *
 * This module is types only, so a host test can load it through type stripping.
 * The mapping onto the wire `UdfType` tags lives in `micro.ts`.
 */

import type { Infer, TypeDescriptor } from '@voml/vos';

/** Phase-1 scalar kinds on the YY wire micro path, aligned with `yydb-udf`. */
export type UdfScalarKind = 'i64' | 'bool' | 'text' | 'null';

/**
 * VOS descriptors accepted at the Phase-1 host micro boundary.
 *
 * Drawn from the frozen VOS type algebra, so `v.utf16()`, `v.optional(…)`,
 * `v.list(…)` and every non-scalar descriptor are rejected before they reach
 * the wire.
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

/** Invocation mode stored with the definition for future batch host calls. */
export type MicroInvocationMode = 'scalar' | 'batch';

