/**
 * Real OPFS acceptance inside a dedicated worker.
 *
 * Sync access handles only exist in a worker, so the whole durable path runs
 * here: the host opens the file handles, hands the sync handles to the WASM
 * core, and a second reopen proves the bytes survived the first session.
 */
import { initWasm, openPersistentSessionWithHandles, type PersistentHandles } from "@yydb/yydb-unknown-wasm32";

interface WorkerResult {
    ok: boolean;
    error?: string;
}

const YYDB = "light-db-opfs-probe.yydb";
const YYDX = "light-db-opfs-probe.yydx";

type SyncHandleCapable = {
    createSyncAccessHandle(): Promise<unknown>;
};

type CloseCapable = {
    close(): void;
};

/** Handles plus the release path: OPFS refuses a second handle while one is open. */
interface OpenedHandles {
    handles: PersistentHandles;
    close(): void;
}

async function root(): Promise<FileSystemDirectoryHandle> {
    // `getDirectory` is present in browsers that ship OPFS.
    return (navigator.storage as unknown as { getDirectory(): Promise<FileSystemDirectoryHandle> }).getDirectory();
}

async function syncHandle(name: string): Promise<unknown> {
    const directory = await root();
    const handle = (await directory.getFileHandle(name, { create: true })) as unknown as SyncHandleCapable;
    return handle.createSyncAccessHandle();
}

async function handlesFor(name: string): Promise<OpenedHandles> {
    const main = await syncHandle(name);
    const wal = await syncHandle(`${name}-wal`);
    const shm = await syncHandle(`${name}-shm`);
    return {
        handles: { main, wal, shm },
        close: () => {
            for (const handle of [main, wal, shm]) {
                (handle as CloseCapable).close();
            }
        },
    };
}

async function reset(): Promise<void> {
    const directory = await root();
    for (const name of [YYDB, `${YYDB}-wal`, `${YYDB}-shm`, YYDX, `${YYDX}-wal`, `${YYDX}-shm`]) {
        try {
            await directory.removeEntry(name);
        } catch {
            // A missing entry is already the state we want.
        }
    }
}

function envelope(raw: string): Record<string, unknown> {
    return JSON.parse(raw) as Record<string, unknown>;
}

function assertOk(raw: string, operation: string): Record<string, unknown> {
    const parsed = envelope(raw);
    if (parsed.ok !== true) {
        throw new Error(`${operation} failed: ${JSON.stringify(parsed.error)}`);
    }
    return parsed;
}

async function run(): Promise<void> {
    await initWasm();
    await reset();

    // 1. A first session publishes a schema and one row, then closes cleanly.
    {
        const opened = await handlesFor(YYDB);
        const session = openPersistentSessionWithHandles(YYDB, opened.handles);
        assertOk(session.ensureSchema("table Probe { @@id: i64, note: utf8 }"), "ensureSchema");
        assertOk(session.put("theme", new TextEncoder().encode("dark")), "put");
        if (session.generation < 1) {
            throw new Error(`a durable publish must advance the generation, got ${session.generation}`);
        }
        session.close();
        opened.close();
    }

    // 2. A brand-new set of handles over the same OPFS files is a restart.
    {
        const opened = await handlesFor(YYDB);
        const session = openPersistentSessionWithHandles(YYDB, opened.handles);
        const raw = envelope(session.get("theme"));
        if (raw.ok !== true) {
            throw new Error(`a committed row must survive the reopen: ${JSON.stringify(raw)}`);
        }
        const value = new Uint8Array(raw.value as number[] | ArrayLike<number>);
        if (new TextDecoder().decode(value) !== "dark") {
            throw new Error(`unexpected persisted value: ${new TextDecoder().decode(value)}`);
        }
        if (session.generation < 1) {
            throw new Error("the reopened session must see the published generation");
        }
        session.close();
        opened.close();
    }

    // 3. `.yydx` without a host blob directory is a structured refusal, not a
    //    silent fallback to a single-file layout.
    {
        const opened = await handlesFor(YYDX);
        let refused = false;
        try {
            openPersistentSessionWithHandles(YYDX, opened.handles);
        } catch (error) {
            refused = String(error).includes("blob_directory");
        } finally {
            opened.close();
        }
        if (!refused) {
            throw new Error("a `.yydx` volume must refuse without a blob directory");
        }
    }

    // 4. The resolved directory facade the host builds from `values()` and
    //    `removeEntry()` satisfies the capability.
    {
        const opened = await handlesFor(YYDX);
        const directory = { list: () => [] as string[], remove: () => {} };
        const session = openPersistentSessionWithHandles(YYDX, { ...opened.handles, directory });
        session.close();
        opened.close();
    }
}

self.onmessage = () => {
    run()
        .then(() => self.postMessage({ ok: true } satisfies WorkerResult))
        .catch((error: unknown) => {
            const message = error instanceof Error ? error.message : String(error);
            self.postMessage({ ok: false, error: message } satisfies WorkerResult);
        });
};
