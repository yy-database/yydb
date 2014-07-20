import type { InferUdfArgs, InferUdfReturn, MicroEffect, MicroInvocationMode, UdfScalarDescriptor } from './udf-types.js';
import type { AnyTypeDescriptor } from '@voml/vos';

/** Value shape accepted and produced on the wire micro path. */
type MicroWireValue = string | number | boolean | null;

/** Wire kinds a VOS descriptor can map onto (`UdfType::Null` is never a type here). */
type UdfWireKind = 'i64' | 'bool' | 'text';

/**
 * Maps a VOS descriptor onto its wire `UdfType` tag.
 *
 * Accepts any VOS descriptor so the runtime layer can reject a kind that
 * slipped past the type layer; throws for everything outside the Phase-1
 * subset instead of letting it reach the engine.
 */
export function udfWireKind(descriptor: AnyTypeDescriptor): UdfWireKind {
    switch (descriptor.kind) {
        case 'i64':
            return 'i64';
        case 'bool':
            return 'bool';
        case 'utf8':
            return 'text';
        default:
            throw new Error(`unsupported micro type descriptor: ${descriptor.kind}`);
    }
}

/** Input accepted by [`Database.defineMicro`](./database.ts). */
export interface TsMicroDefinitionInput<TArgs extends readonly UdfScalarDescriptor[], TReturn extends UdfScalarDescriptor> {
    readonly name: string;
    readonly version: number;
    readonly args: TArgs;
    readonly returns: TReturn;
    readonly effect: MicroEffect;
    readonly deterministic: boolean;
    readonly mode?: MicroInvocationMode;
    readonly fn: (...args: InferUdfArgs<TArgs>) => InferUdfReturn<TReturn>;
}

/** Opaque definition produced by `defineMicro` before session registration. */
export interface DefinedMicro<
    TArgs extends readonly UdfScalarDescriptor[] = readonly UdfScalarDescriptor[],
    TReturn extends UdfScalarDescriptor = UdfScalarDescriptor,
> {
    readonly name: string;
    readonly version: number;
    readonly args: TArgs;
    readonly returns: TReturn;
    readonly effect: MicroEffect;
    readonly deterministic: boolean;
    readonly mode: MicroInvocationMode;
    readonly placement: 'host';
    readonly fn: (...args: InferUdfArgs<TArgs>) => InferUdfReturn<TReturn>;
}

/** Handle returned after `registerMicro` installs a micro into the session. */
export interface MicroHandle {
    readonly name: string;
    readonly version: number;
    readonly functionId: string;
    readonly hostId: number;
    /** Monotonic host implementation version for opaque handle checks. */
    readonly implementationVersion: number;
}

type AnyDefinedMicro = DefinedMicro<readonly UdfScalarDescriptor[], UdfScalarDescriptor>;

interface RegisteredMicro {
    readonly handle: MicroHandle;
    readonly definition: AnyDefinedMicro;
}

let nextHostId = 1;

/**
 * Process-local registry for JS functions behind opaque host handles.
 * Rust keeps `host_id + function_id + version`, not the closure.
 */
export class MicroSessionRegistry {
    readonly hostId: number;
    private nextFunctionId = 1;
    private readonly byKey = new Map<string, RegisteredMicro>();
    private readonly byFunctionId = new Map<string, RegisteredMicro>();

    constructor(hostId = nextHostId++) {
        this.hostId = hostId;
    }

    /** Installs a defined micro into the current session registry. */
    register<TArgs extends readonly UdfScalarDescriptor[], TReturn extends UdfScalarDescriptor>(
        definition: DefinedMicro<TArgs, TReturn>,
    ): MicroHandle {
        const key = logicalKey(definition.name, definition.version);
        if (this.byKey.has(key)) {
            throw new Error(`micro already registered: ${definition.name}@${definition.version}`);
        }
        const functionId = String(this.nextFunctionId++);
        const handle: MicroHandle = {
            name: definition.name,
            version: definition.version,
            functionId,
            hostId: this.hostId,
            implementationVersion: 1,
        };
        const entry: RegisteredMicro = { handle, definition: definition as AnyDefinedMicro };
        this.byKey.set(key, entry);
        this.byFunctionId.set(functionId, entry);
        return handle;
    }

    /** Returns a registered micro by opaque host handle. */
    resolve(handle: MicroHandle): AnyDefinedMicro | undefined {
        if (handle.hostId !== this.hostId) {
            return undefined;
        }
        return this.byFunctionId.get(handle.functionId)?.definition;
    }

    /** Invokes a registered micro from a wire `MicroHostInvoke` payload. */
    invokeFromWire(payload: { hostId: number; handleVersion: number; functionId: string; args: readonly unknown[] }): MicroWireValue {
        const handle: MicroHandle = {
            hostId: payload.hostId,
            functionId: payload.functionId,
            implementationVersion: payload.handleVersion,
            name: '',
            version: 0,
        };
        const definition = this.resolve(handle);
        if (!definition) {
            throw new Error(`micro not found: ${payload.functionId}`);
        }
        const args = definition.args.map((descriptor, index) => wireToBoundary(descriptor, payload.args[index]));
        return boundaryToWire(definition.returns, this.invokeScalar(handle, args));
    }

    /** Invokes a registered scalar micro (host adapter entry point). */
    invokeScalar(handle: MicroHandle, args: unknown[]): unknown {
        const definition = this.resolve(handle);
        if (!definition) {
            throw new Error(`micro not found: ${handle.functionId}`);
        }
        if (definition.mode !== 'scalar') {
            throw new Error(`micro ${definition.name} is not scalar mode`);
        }
        if (args.length !== definition.args.length) {
            throw new Error(`micro ${definition.name} arity mismatch: expected ${definition.args.length}, got ${args.length}`);
        }
        definition.args.forEach((descriptor, index) => {
            validateValue(descriptor, args[index], `micro ${definition.name} arg ${index}`);
        });
        const result = definition.fn(...(args as []));
        validateValue(definition.returns, result, `micro ${definition.name} return`);
        return result;
    }

    /** Lists logical names currently registered in this session. */
    list(): string[] {
        return [...this.byKey.values()].map((entry) => entry.definition.name).sort();
    }
}

/** Builds a session-local micro definition without registering it. */
export function defineMicro<TArgs extends readonly UdfScalarDescriptor[], TReturn extends UdfScalarDescriptor>(
    input: TsMicroDefinitionInput<TArgs, TReturn>,
): DefinedMicro<TArgs, TReturn> {
    if (!input.name.trim()) {
        throw new Error('micro name must not be empty');
    }
    if (!Number.isInteger(input.version) || input.version <= 0) {
        throw new Error('micro version must be a positive integer');
    }
    if (input.effect !== 'pure') {
        throw new Error('TS micro effect must be pure in Phase 1');
    }
    if (!input.deterministic) {
        throw new Error('TS micro must be deterministic in Phase 1');
    }
    if (typeof input.fn !== 'function') {
        throw new Error('micro fn must be a function');
    }
    // Reject descriptors outside the Phase-1 subset while the definition is
    // still session-local, instead of at register time.
    input.args.forEach((descriptor, index) => {
        assertSupportedDescriptor(descriptor, `micro ${input.name} arg ${index}`);
    });
    assertSupportedDescriptor(input.returns, `micro ${input.name} return`);
    return {
        name: input.name,
        version: input.version,
        args: input.args,
        returns: input.returns,
        effect: input.effect,
        deterministic: input.deterministic,
        mode: input.mode ?? 'scalar',
        placement: 'host',
        fn: input.fn,
    };
}

/** Stable 32-byte fingerprint for a logical micro name and version. */
export function microFingerprint(name: string, version: number): Uint8Array {
    const bytes = new Uint8Array(32);
    const seed = `${name}@${version}`;
    for (let index = 0; index < bytes.byteLength; index += 1) {
        bytes[index] = seed.charCodeAt(index % seed.length) & 0xff;
    }
    return bytes;
}

function logicalKey(name: string, version: number): string {
    return `${name}@${version}`;
}

function assertSupportedDescriptor(descriptor: UdfScalarDescriptor, label: string): void {
    try {
        udfWireKind(descriptor);
    } catch {
        throw new Error(`${label} uses a descriptor outside the Phase-1 micro subset`);
    }
}

/** Boundary value a VOS descriptor carries in the host runtime. */
function validateValue(descriptor: UdfScalarDescriptor, value: unknown, label: string): void {
    switch (udfWireKind(descriptor)) {
        case 'i64':
            if (typeof value !== 'bigint') {
                throw new Error(`${label} expects VOS \`i64\` (a bigint)`);
            }
            break;
        case 'bool':
            if (typeof value !== 'boolean') {
                throw new Error(`${label} expects VOS \`bool\``);
            }
            break;
        case 'text':
            if (typeof value !== 'string') {
                throw new Error(`${label} expects VOS \`utf8\` (a string)`);
            }
            break;
    }
}

/**
 * Wire value to boundary value. The YY wire carries `i64` as a JS number, so
 * it becomes the VOS `i64` boundary value here rather than in author code.
 */
function wireToBoundary(descriptor: UdfScalarDescriptor, value: unknown): unknown {
    if (udfWireKind(descriptor) === 'i64' && typeof value === 'number' && Number.isSafeInteger(value)) {
        return BigInt(value);
    }
    return value;
}

/** Boundary value back to the wire subset; `i64` must still fit a JS number. */
function boundaryToWire(descriptor: UdfScalarDescriptor, value: unknown): MicroWireValue {
    if (udfWireKind(descriptor) === 'i64' && typeof value === 'bigint') {
        if (value > BigInt(Number.MAX_SAFE_INTEGER) || value < BigInt(Number.MIN_SAFE_INTEGER)) {
            throw new Error(`micro return i64 ${value} does not fit the wire number path`);
        }
        return Number(value);
    }
    return value as MicroWireValue;
}
