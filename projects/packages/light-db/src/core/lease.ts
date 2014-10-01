/**
 * 单写者租约
 *
 * 浏览器多标签页共享同一份 OPFS 数据，写者必须互斥。
 * 首选 Web Locks，缺失时不静默退化成多写者。
 */

import { LightDbError, LightDbErrorCodes } from "../errors";

/** 写租约 */
export interface WriterLease {
    /** 释放租约 */
    release(): Promise<void>;
}

/** 租约模式 */
export type LeaseMode = "web-locks" | "none";

/** Web Locks 结构化类型，避免引入 DOM lib */
interface LockManagerLike {
    request(
        name: string,
        options: { mode: "exclusive"; ifAvailable: true },
        callback: (lock: unknown) => Promise<void>,
    ): Promise<void>;
}

/**
 * 获取写租约
 *
 * @param databaseName 数据库名，用作锁名
 * @param mode `web-locks` 走 Web Locks，`none` 表示调用方自行保证单写者（测试或单上下文）
 * @param locks 可注入的锁实现，`null` 表示显式缺失，省略时读取宿主
 * @returns 写租约
 * @throws LightDbError `yydb.lease.unavailable` 当环境没有 Web Locks 或锁已被占用
 */
export async function acquireWriterLease(
    databaseName: string,
    mode: LeaseMode,
    locks?: LockManagerLike | null,
): Promise<WriterLease> {
    if (mode === "none") {
        return { release: async () => undefined };
    }

    const manager = locks === undefined ? getLockManager() : locks;
    if (!manager) {
        throw new LightDbError(
            LightDbErrorCodes.leaseUnavailable,
            "Web Locks is unavailable, refusing to open a second writer",
            { operation: "acquireWriteLease" },
        );
    }

    let releaseHold: () => void = () => undefined;
    const hold = new Promise<void>((resolve) => {
        releaseHold = resolve;
    });

    let settled = false;
    let released = false;
    let completion: Promise<void> = Promise.resolve();

    const acquired = new Promise<void>((resolve, reject) => {
        completion = manager
            .request(`yydb:${databaseName}`, { mode: "exclusive", ifAvailable: true }, async (lock) => {
                if (!lock) {
                    if (!settled) {
                        settled = true;
                        reject(
                            new LightDbError(LightDbErrorCodes.leaseUnavailable, "database is opened by another writer", {
                                operation: "acquireWriteLease",
                            }),
                        );
                    }
                    return;
                }

                if (!settled) {
                    settled = true;
                    resolve();
                }

                await hold;
            })
            .catch((error: unknown) => {
                if (!settled) {
                    settled = true;
                    reject(
                        new LightDbError(LightDbErrorCodes.leaseUnavailable, "failed to acquire the writer lease", {
                            operation: "acquireWriteLease",
                            cause: error,
                        }),
                    );
                }
            });
    });

    await acquired;

    return {
        release: async () => {
            if (released) {
                return;
            }
            released = true;
            releaseHold();
            await completion.catch(() => undefined);
        },
    };
}

/**
 * 取得宿主的 Web Locks 实现
 */
function getLockManager(): LockManagerLike | null {
    const navigatorLike = (globalThis as { navigator?: { locks?: LockManagerLike } }).navigator;
    return navigatorLike?.locks ?? null;
}
