/** Phase-1 scalar UDF wire kinds aligned with `yydb-udf`. */
export type UdfScalarKind = 'i64' | 'bool' | 'text' | 'null';

/** Runtime type descriptor used in `defineMicro` contracts. */
export interface UdfTypeDescriptor<T = unknown> {
    readonly kind: UdfScalarKind;
    /** Phantom type for author-side inference only. */
    readonly _type?: T;
}

/** `i64` argument or return descriptor. */
export function i64(): UdfTypeDescriptor<number> {
    return { kind: 'i64' };
}

/** `bool` argument or return descriptor. */
export function bool(): UdfTypeDescriptor<boolean> {
    return { kind: 'bool' };
}

/** `text` argument or return descriptor. */
export function text(): UdfTypeDescriptor<string> {
    return { kind: 'text' };
}

/** `null` return descriptor. */
export function nullType(): UdfTypeDescriptor<null> {
    return { kind: 'null' };
}

/** Maps descriptor tuple types to positional argument types. */
export type InferUdfArgs<T extends readonly UdfTypeDescriptor[]> = {
    [K in keyof T]: T[K] extends UdfTypeDescriptor<infer V> ? V : never;
};

/** Maps a return descriptor to its value type. */
export type InferUdfReturn<T extends UdfTypeDescriptor> = T extends UdfTypeDescriptor<infer V> ? V : never;

/** Session micro effect surface (Phase 1: pure only). */
export type MicroEffect = 'pure';

/** Invocation mode stored with the definition for future batch host calls. */
export type MicroInvocationMode = 'scalar' | 'batch';
