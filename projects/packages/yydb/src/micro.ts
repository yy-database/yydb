import type {
    InferUdfArgs,
    InferUdfReturn,
    MicroEffect,
    MicroInvocationMode,
    UdfTypeDescriptor,
} from "./udf-types.js";

/** Input accepted by [`Database.defineMicro`](./database.ts). */
export interface TsMicroDefinitionInput<
    TArgs extends readonly UdfTypeDescriptor[],
    TReturn extends UdfTypeDescriptor,
> {
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
    TArgs extends readonly UdfTypeDescriptor[] = readonly UdfTypeDescriptor[],
    TReturn extends UdfTypeDescriptor = UdfTypeDescriptor,
> {
    readonly name: string;
    readonly version: number;
    readonly args: TArgs;
    readonly returns: TReturn;
    readonly effect: MicroEffect;
    readonly deterministic: boolean;
    readonly mode: MicroInvocationMode;
    readonly placement: "host";
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

type AnyDefinedMicro = DefinedMicro<
    readonly UdfTypeDescriptor[],
    UdfTypeDescriptor
>;

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
    register<TArgs extends readonly UdfTypeDescriptor[], TReturn extends UdfTypeDescriptor>(
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
    invokeFromWire(payload: {
        hostId: number;
        handleVersion: number;
        functionId: string;
        args: readonly unknown[];
    }): string | number | boolean | null {
        const handle: MicroHandle = {
            hostId: payload.hostId,
            functionId: payload.functionId,
            implementationVersion: payload.handleVersion,
            name: "",
            version: 0,
        };
        return this.invokeScalar(handle, [...payload.args]) as
            | string
            | number
            | boolean
            | null;
    }

    /** Invokes a registered scalar micro (host adapter entry point). */
    invokeScalar(handle: MicroHandle, args: unknown[]): unknown {
        const definition = this.resolve(handle);
        if (!definition) {
            throw new Error(`micro not found: ${handle.functionId}`);
        }
        if (definition.mode !== "scalar") {
            throw new Error(`micro ${definition.name} is not scalar mode`);
        }
        if (args.length !== definition.args.length) {
            throw new Error(
                `micro ${definition.name} arity mismatch: expected ${definition.args.length}, got ${args.length}`,
            );
        }
        validateArgTypes(definition, args);
        const result = definition.fn(...(args as []));
        validateReturnType(definition.returns, result);
        return result;
    }

    /** Lists logical names currently registered in this session. */
    list(): string[] {
        return [...this.byKey.values()].map((entry) => entry.definition.name).sort();
    }
}

/** Builds a session-local micro definition without registering it. */
export function defineMicro<
    TArgs extends readonly UdfTypeDescriptor[],
    TReturn extends UdfTypeDescriptor,
>(input: TsMicroDefinitionInput<TArgs, TReturn>): DefinedMicro<TArgs, TReturn> {
    if (!input.name.trim()) {
        throw new Error("micro name must not be empty");
    }
    if (!Number.isInteger(input.version) || input.version <= 0) {
        throw new Error("micro version must be a positive integer");
    }
    if (input.effect !== "pure") {
        throw new Error("TS micro effect must be pure in Phase 1");
    }
    if (!input.deterministic) {
        throw new Error("TS micro must be deterministic in Phase 1");
    }
    if (typeof input.fn !== "function") {
        throw new Error("micro fn must be a function");
    }
    return {
        name: input.name,
        version: input.version,
        args: input.args,
        returns: input.returns,
        effect: input.effect,
        deterministic: input.deterministic,
        mode: input.mode ?? "scalar",
        placement: "host",
        fn: input.fn,
    };
}

function logicalKey(name: string, version: number): string {
    return `${name}@${version}`;
}

function validateArgTypes(definition: AnyDefinedMicro, args: unknown[]): void {
    for (let index = 0; index < definition.args.length; index += 1) {
        const kind = definition.args[index]?.kind;
        const value = args[index];
        switch (kind) {
            case "i64":
                if (typeof value !== "number" || !Number.isInteger(value)) {
                    throw new Error(`micro ${definition.name} arg ${index} expects i64`);
                }
                break;
            case "bool":
                if (typeof value !== "boolean") {
                    throw new Error(`micro ${definition.name} arg ${index} expects bool`);
                }
                break;
            case "text":
                if (typeof value !== "string") {
                    throw new Error(`micro ${definition.name} arg ${index} expects text`);
                }
                break;
            case "null":
                if (value !== null) {
                    throw new Error(`micro ${definition.name} arg ${index} expects null`);
                }
                break;
            default:
                throw new Error(`unsupported micro arg kind: ${kind}`);
        }
    }
}

function validateReturnType(descriptor: UdfTypeDescriptor, value: unknown): void {
    switch (descriptor.kind) {
        case "i64":
            if (typeof value !== "number" || !Number.isInteger(value)) {
                throw new Error("micro return expects i64");
            }
            break;
        case "bool":
            if (typeof value !== "boolean") {
                throw new Error("micro return expects bool");
            }
            break;
        case "text":
            if (typeof value !== "string") {
                throw new Error("micro return expects text");
            }
            break;
        case "null":
            if (value !== null) {
                throw new Error("micro return expects null");
            }
            break;
        default:
            throw new Error(`unsupported micro return kind: ${descriptor.kind}`);
    }
}
