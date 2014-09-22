/**
 * 读写锁实现
 * 允许多个读查询并发，写操作独占
 */

import { RWLockOptions, RWLockStats } from "./types";

/**
 * 等待队列项
 */
interface Waiter {
    resolve: () => void;
    reject: (error: Error) => void;
    type: "read" | "write";
    timestamp: number;
}

/**
 * 读锁句柄
 * 用于自动释放锁
 */
export class ReadLockHandle {
    private released: boolean = false;
    private readonly releaseFn: () => void;

    constructor(releaseFn: () => void) {
        this.releaseFn = releaseFn;
    }

    /**
     * 释放读锁
     */
    public release(): void {
        if (!this.released) {
            this.released = true;
            this.releaseFn();
        }
    }

    /**
     * 检查是否已释放
     */
    public isReleased(): boolean {
        return this.released;
    }
}

/**
 * 写锁句柄
 * 用于自动释放锁
 */
export class WriteLockHandle {
    private released: boolean = false;
    private readonly releaseFn: () => void;

    constructor(releaseFn: () => void) {
        this.releaseFn = releaseFn;
    }

    /**
     * 释放写锁
     */
    public release(): void {
        if (!this.released) {
            this.released = true;
            this.releaseFn();
        }
    }

    /**
     * 检查是否已释放
     */
    public isReleased(): boolean {
        return this.released;
    }
}

/**
 * 读写锁
 * 支持多读单写，可选公平模式
 */
export class RWLock {
    private readers: number;
    private writer: boolean;
    private writeWaiters: Waiter[];
    private readWaiters: Waiter[];
    private readonly timeout: number;
    private readonly fair: boolean;
    private totalReadAcquires: number;
    private totalWriteAcquires: number;

    /**
     * 创建读写锁
     * @param options 配置选项
     */
    constructor(options: RWLockOptions = {}) {
        this.readers = 0;
        this.writer = false;
        this.writeWaiters = [];
        this.readWaiters = [];
        this.timeout = options.timeout ?? 0;
        this.fair = options.fair ?? false;
        this.totalReadAcquires = 0;
        this.totalWriteAcquires = 0;
    }

    /**
     * 获取读锁
     * @returns 读锁句柄
     */
    public async acquireRead(): Promise<ReadLockHandle> {
        if (this.canAcquireRead()) {
            this.readers++;
            this.totalReadAcquires++;
            return new ReadLockHandle(() => this.releaseRead());
        }

        return new Promise<ReadLockHandle>((resolve, reject) => {
            const waiter: Waiter = {
                resolve: () => {
                    this.readers++;
                    this.totalReadAcquires++;
                    resolve(new ReadLockHandle(() => this.releaseRead()));
                },
                reject,
                type: "read",
                timestamp: Date.now(),
            };

            this.readWaiters.push(waiter);

            if (this.timeout > 0) {
                setTimeout(() => {
                    const index = this.readWaiters.indexOf(waiter);
                    if (index !== -1) {
                        this.readWaiters.splice(index, 1);
                        reject(new Error(`Read lock acquisition timeout after ${this.timeout}ms`));
                    }
                }, this.timeout);
            }
        });
    }

    /**
     * 尝试获取读锁（非阻塞）
     * @returns 读锁句柄或 null
     */
    public tryAcquireRead(): ReadLockHandle | null {
        if (this.canAcquireRead()) {
            this.readers++;
            this.totalReadAcquires++;
            return new ReadLockHandle(() => this.releaseRead());
        }
        return null;
    }

    /**
     * 检查是否可以获取读锁
     */
    private canAcquireRead(): boolean {
        if (this.writer) {
            return false;
        }

        if (this.fair && this.writeWaiters.length > 0) {
            return false;
        }

        return true;
    }

    /**
     * 获取写锁
     * @returns 写锁句柄
     */
    public async acquireWrite(): Promise<WriteLockHandle> {
        if (this.canAcquireWrite()) {
            this.writer = true;
            this.totalWriteAcquires++;
            return new WriteLockHandle(() => this.releaseWrite());
        }

        return new Promise<WriteLockHandle>((resolve, reject) => {
            const waiter: Waiter = {
                resolve: () => {
                    this.writer = true;
                    this.totalWriteAcquires++;
                    resolve(new WriteLockHandle(() => this.releaseWrite()));
                },
                reject,
                type: "write",
                timestamp: Date.now(),
            };

            this.writeWaiters.push(waiter);

            if (this.timeout > 0) {
                setTimeout(() => {
                    const index = this.writeWaiters.indexOf(waiter);
                    if (index !== -1) {
                        this.writeWaiters.splice(index, 1);
                        reject(new Error(`Write lock acquisition timeout after ${this.timeout}ms`));
                    }
                }, this.timeout);
            }
        });
    }

    /**
     * 尝试获取写锁（非阻塞）
     * @returns 写锁句柄或 null
     */
    public tryAcquireWrite(): WriteLockHandle | null {
        if (this.canAcquireWrite()) {
            this.writer = true;
            this.totalWriteAcquires++;
            return new WriteLockHandle(() => this.releaseWrite());
        }
        return null;
    }

    /**
     * 检查是否可以获取写锁
     */
    private canAcquireWrite(): boolean {
        if (this.writer || this.readers > 0) {
            return false;
        }

        return true;
    }

    /**
     * 释放读锁
     */
    private releaseRead(): void {
        if (this.readers > 0) {
            this.readers--;
        }

        this.wakeWaiters();
    }

    /**
     * 释放写锁
     */
    private releaseWrite(): void {
        this.writer = false;
        this.wakeWaiters();
    }

    /**
     * 唤醒等待者
     */
    private wakeWaiters(): void {
        if (this.fair) {
            if (!this.writer && this.writeWaiters.length > 0 && this.readers === 0) {
                const waiter = this.writeWaiters.shift();
                if (waiter) {
                    waiter.resolve();
                    return;
                }
            }

            if (!this.writer && this.readWaiters.length > 0 && this.writeWaiters.length === 0) {
                while (this.readWaiters.length > 0) {
                    const waiter = this.readWaiters.shift();
                    if (waiter) {
                        waiter.resolve();
                    }
                }
            }
        } else {
            if (!this.writer && this.writeWaiters.length > 0 && this.readers === 0) {
                const waiter = this.writeWaiters.shift();
                if (waiter) {
                    waiter.resolve();
                    return;
                }
            }

            if (!this.writer && this.readWaiters.length > 0) {
                while (this.readWaiters.length > 0) {
                    const waiter = this.readWaiters.shift();
                    if (waiter) {
                        waiter.resolve();
                    }
                }
            }
        }
    }

    /**
     * 使用读锁执行函数
     * @param fn 要执行的函数
     * @returns 函数返回值
     */
    public async withRead<T>(fn: () => Promise<T>): Promise<T> {
        const handle = await this.acquireRead();
        try {
            return await fn();
        } finally {
            handle.release();
        }
    }

    /**
     * 使用写锁执行函数
     * @param fn 要执行的函数
     * @returns 函数返回值
     */
    public async withWrite<T>(fn: () => Promise<T>): Promise<T> {
        const handle = await this.acquireWrite();
        try {
            return await fn();
        } finally {
            handle.release();
        }
    }

    /**
     * 使用读锁执行同步函数
     * @param fn 要执行的函数
     * @returns 函数返回值
     */
    public withReadSync<T>(fn: () => T): T {
        const handle = this.tryAcquireRead();
        if (!handle) {
            throw new Error("Cannot acquire read lock synchronously");
        }
        try {
            return fn();
        } finally {
            handle.release();
        }
    }

    /**
     * 使用写锁执行同步函数
     * @param fn 要执行的函数
     * @returns 函数返回值
     */
    public withWriteSync<T>(fn: () => T): T {
        const handle = this.tryAcquireWrite();
        if (!handle) {
            throw new Error("Cannot acquire write lock synchronously");
        }
        try {
            return fn();
        } finally {
            handle.release();
        }
    }

    /**
     * 获取统计信息
     */
    public getStats(): RWLockStats {
        return {
            readLockCount: this.readers,
            writeLockCount: this.writer ? 1 : 0,
            pendingReadCount: this.readWaiters.length,
            pendingWriteCount: this.writeWaiters.length,
            totalReadAcquires: this.totalReadAcquires,
            totalWriteAcquires: this.totalWriteAcquires,
        };
    }

    /**
     * 检查是否有等待者
     */
    public hasWaiters(): boolean {
        return this.readWaiters.length > 0 || this.writeWaiters.length > 0;
    }

    /**
     * 检查是否被读锁定
     */
    public isReadLocked(): boolean {
        return this.readers > 0;
    }

    /**
     * 检查是否被写锁定
     */
    public isWriteLocked(): boolean {
        return this.writer;
    }

    /**
     * 检查是否被锁定
     */
    public isLocked(): boolean {
        return this.writer || this.readers > 0;
    }

    /**
     * 强制释放所有锁（危险操作，仅用于特殊场景）
     */
    public forceUnlock(): void {
        this.readers = 0;
        this.writer = false;

        const allWaiters = [...this.writeWaiters, ...this.readWaiters];
        this.writeWaiters = [];
        this.readWaiters = [];

        for (const waiter of allWaiters) {
            waiter.reject(new Error("Lock was force unlocked"));
        }
    }
}

/**
 * 读写锁管理器
 * 管理多个命名锁
 */
export class RWLockManager {
    private locks: Map<string, RWLock>;
    private defaultOptions: RWLockOptions;

    /**
     * 创建读写锁管理器
     * @param defaultOptions 默认配置
     */
    constructor(defaultOptions: RWLockOptions = {}) {
        this.locks = new Map();
        this.defaultOptions = defaultOptions;
    }

    /**
     * 获取或创建锁
     * @param name 锁名称
     * @param options 配置选项
     */
    public getLock(name: string, options?: RWLockOptions): RWLock {
        let lock = this.locks.get(name);

        if (!lock) {
            lock = new RWLock({
                ...this.defaultOptions,
                ...options,
            });
            this.locks.set(name, lock);
        }

        return lock;
    }

    /**
     * 获取读锁
     * @param name 锁名称
     * @returns 读锁句柄
     */
    public async acquireRead(name: string): Promise<ReadLockHandle> {
        return this.getLock(name).acquireRead();
    }

    /**
     * 获取写锁
     * @param name 锁名称
     * @returns 写锁句柄
     */
    public async acquireWrite(name: string): Promise<WriteLockHandle> {
        return this.getLock(name).acquireWrite();
    }

    /**
     * 使用读锁执行函数
     * @param name 锁名称
     * @param fn 要执行的函数
     */
    public async withRead<T>(name: string, fn: () => Promise<T>): Promise<T> {
        return this.getLock(name).withRead(fn);
    }

    /**
     * 使用写锁执行函数
     * @param name 锁名称
     * @param fn 要执行的函数
     */
    public async withWrite<T>(name: string, fn: () => Promise<T>): Promise<T> {
        return this.getLock(name).withWrite(fn);
    }

    /**
     * 获取所有锁名称
     */
    public getLockNames(): string[] {
        return Array.from(this.locks.keys());
    }

    /**
     * 获取锁统计信息
     * @param name 锁名称
     */
    public getStats(name: string): RWLockStats | undefined {
        return this.locks.get(name)?.getStats();
    }

    /**
     * 获取所有锁统计信息
     */
    public getAllStats(): Map<string, RWLockStats> {
        const stats = new Map<string, RWLockStats>();

        for (const [name, lock] of this.locks) {
            stats.set(name, lock.getStats());
        }

        return stats;
    }

    /**
     * 删除锁
     * @param name 锁名称
     */
    public deleteLock(name: string): boolean {
        const lock = this.locks.get(name);
        if (lock && lock.isLocked()) {
            return false;
        }
        return this.locks.delete(name);
    }

    /**
     * 清空所有锁
     */
    public clear(): void {
        for (const lock of this.locks.values()) {
            if (lock.isLocked()) {
                throw new Error("Cannot clear locks while some are still held");
            }
        }
        this.locks.clear();
    }
}

/**
 * 可重入读写锁
 * 支持同一线程多次获取同一锁
 */
export class ReentrantRWLock {
    private lock: RWLock;
    private readOwners: Map<number, number>;
    private writeOwner: number | null;
    private currentId: number;

    /**
     * 创建可重入读写锁
     * @param options 配置选项
     */
    constructor(options: RWLockOptions = {}) {
        this.lock = new RWLock(options);
        this.readOwners = new Map();
        this.writeOwner = null;
        this.currentId = 0;
    }

    /**
     * 生成当前上下文 ID
     * 在实际应用中，应该使用线程 ID 或其他唯一标识
     */
    private getContextId(): number {
        return ++this.currentId;
    }

    /**
     * 获取读锁
     */
    public async acquireRead(): Promise<ReadLockHandle> {
        const contextId = this.getContextId();

        const count = this.readOwners.get(contextId) ?? 0;
        if (count > 0) {
            this.readOwners.set(contextId, count + 1);
            return new ReadLockHandle(() => this.releaseRead(contextId));
        }

        if (this.writeOwner === contextId) {
            return new ReadLockHandle(() => this.releaseRead(contextId));
        }

        const handle = await this.lock.acquireRead();
        this.readOwners.set(contextId, 1);

        return new ReadLockHandle(() => {
            this.releaseRead(contextId);
            handle.release();
        });
    }

    /**
     * 释放读锁
     */
    private releaseRead(contextId: number): void {
        const count = this.readOwners.get(contextId);
        if (count !== undefined) {
            if (count <= 1) {
                this.readOwners.delete(contextId);
            } else {
                this.readOwners.set(contextId, count - 1);
            }
        }
    }

    /**
     * 获取写锁
     */
    public async acquireWrite(): Promise<WriteLockHandle> {
        const contextId = this.getContextId();

        if (this.writeOwner === contextId) {
            return new WriteLockHandle(() => this.releaseWrite(contextId));
        }

        const handle = await this.lock.acquireWrite();
        this.writeOwner = contextId;

        return new WriteLockHandle(() => {
            this.releaseWrite(contextId);
            handle.release();
        });
    }

    /**
     * 释放写锁
     */
    private releaseWrite(contextId: number): void {
        if (this.writeOwner === contextId) {
            this.writeOwner = null;
        }
    }

    /**
     * 使用读锁执行函数
     */
    public async withRead<T>(fn: () => Promise<T>): Promise<T> {
        const handle = await this.acquireRead();
        try {
            return await fn();
        } finally {
            handle.release();
        }
    }

    /**
     * 使用写锁执行函数
     */
    public async withWrite<T>(fn: () => Promise<T>): Promise<T> {
        const handle = await this.acquireWrite();
        try {
            return await fn();
        } finally {
            handle.release();
        }
    }

    /**
     * 获取统计信息
     */
    public getStats(): RWLockStats {
        return this.lock.getStats();
    }
}
