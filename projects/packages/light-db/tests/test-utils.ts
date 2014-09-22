/**
 * LightDB 测试工具模块
 * 提供测试运行器和辅助函数
 */

import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";

/**
 * 测试结果接口
 */
export interface TestResult {
    /** 测试名称 */
    name: string;
    /** 是否通过 */
    passed: boolean;
    /** 执行时间（毫秒） */
    duration: number;
    /** 错误信息 */
    error?: string;
}

/**
 * 测试套件结果接口
 */
export interface TestSuiteResult {
    /** 套件名称 */
    suiteName: string;
    /** 测试结果列表 */
    results: TestResult[];
    /** 通过数量 */
    passed: number;
    /** 失败数量 */
    failed: number;
    /** 总执行时间 */
    duration: number;
}

/**
 * 测试函数类型
 */
export type TestFunction = () => Promise<void> | void;

/**
 * 测试用例定义
 */
export interface TestCase {
    /** 测试名称 */
    name: string;
    /** 测试函数 */
    fn: TestFunction;
    /** 是否跳过 */
    skip?: boolean;
    /** 是否独占运行 */
    only?: boolean;
}

/**
 * 测试套件类
 * 提供测试组织和执行功能
 */
export class TestSuite {
    private readonly name: string;
    private readonly tests: TestCase[] = [];
    private beforeEachFn: TestFunction | null = null;
    private afterEachFn: TestFunction | null = null;
    private beforeAllFn: TestFunction | null = null;
    private afterAllFn: TestFunction | null = null;

    constructor(name: string) {
        this.name = name;
    }

    /**
     * 添加测试用例
     */
    public test(name: string, fn: TestFunction): this {
        this.tests.push({ name, fn });
        return this;
    }

    /**
     * 跳过的测试
     */
    public skip(name: string, fn: TestFunction): this {
        this.tests.push({ name, fn, skip: true });
        return this;
    }

    /**
     * 独占运行的测试
     */
    public only(name: string, fn: TestFunction): this {
        this.tests.push({ name, fn, only: true });
        return this;
    }

    /**
     * 设置每个测试前的钩子
     */
    public beforeEach(fn: TestFunction): this {
        this.beforeEachFn = fn;
        return this;
    }

    /**
     * 设置每个测试后的钩子
     */
    public afterEach(fn: TestFunction): this {
        this.afterEachFn = fn;
        return this;
    }

    /**
     * 设置所有测试前的钩子
     */
    public beforeAll(fn: TestFunction): this {
        this.beforeAllFn = fn;
        return this;
    }

    /**
     * 设置所有测试后的钩子
     */
    public afterAll(fn: TestFunction): this {
        this.afterAllFn = fn;
        return this;
    }

    /**
     * 运行测试套件
     */
    public async run(): Promise<TestSuiteResult> {
        const results: TestResult[] = [];
        let passed = 0;
        let failed = 0;
        const startTime = Date.now();

        console.log(`\n\x1b[36m=== ${this.name} ===\x1b[0m\n`);

        if (this.beforeAllFn) {
            await this.beforeAllFn();
        }

        const hasOnly = this.tests.some((t) => t.only);
        const testsToRun = hasOnly ? this.tests.filter((t) => t.only) : this.tests;

        for (const testCase of testsToRun) {
            if (testCase.skip) {
                console.log(`  \x1b[90m○ ${testCase.name} (skipped)\x1b[0m`);
                continue;
            }

            const testStartTime = Date.now();
            let testPassed = true;
            let error: string | undefined;

            try {
                if (this.beforeEachFn) {
                    await this.beforeEachFn();
                }

                await testCase.fn();

                if (this.afterEachFn) {
                    await this.afterEachFn();
                }
            } catch (e) {
                testPassed = false;
                error = e instanceof Error ? e.message : String(e);
            }

            const duration = Date.now() - testStartTime;

            if (testPassed) {
                passed++;
                console.log(`  \x1b[32m✓ ${testCase.name}\x1b[0m (${duration}ms)`);
            } else {
                failed++;
                console.log(`  \x1b[31m✗ ${testCase.name}\x1b[0m (${duration}ms)`);
                if (error) {
                    console.log(`    \x1b[31m${error}\x1b[0m`);
                }
            }

            results.push({
                name: testCase.name,
                passed: testPassed,
                duration,
                error,
            });
        }

        if (this.afterAllFn) {
            await this.afterAllFn();
        }

        const totalDuration = Date.now() - startTime;

        console.log(`\n  \x1b[32m${passed} passed\x1b[0m, \x1b[31m${failed} failed\x1b[0m (${totalDuration}ms)\n`);

        return {
            suiteName: this.name,
            results,
            passed,
            failed,
            duration: totalDuration,
        };
    }
}

/**
 * 创建测试套件
 */
export function describe(name: string): TestSuite {
    return new TestSuite(name);
}

/**
 * 断言相等
 */
export function assertEqual<T>(actual: T, expected: T, message?: string): void {
    assert.deepStrictEqual(actual, expected, message);
}

/**
 * 断言不相等
 */
export function assertNotEqual<T>(actual: T, expected: T, message?: string): void {
    assert.notDeepStrictEqual(actual, expected, message);
}

/**
 * 断言为真
 */
export function assertTrue(value: boolean, message?: string): void {
    assert.strictEqual(value, true, message);
}

/**
 * 断言为假
 */
export function assertFalse(value: boolean, message?: string): void {
    assert.strictEqual(value, false, message);
}

/**
 * 断言为空
 */
export function assertNull<T>(value: T | null, message?: string): void {
    assert.strictEqual(value, null, message);
}

/**
 * 断言不为空
 */
export function assertNotNull<T>(value: T | null | undefined, message?: string): void {
    assert.notStrictEqual(value, null, message);
    assert.notStrictEqual(value, undefined, message);
}

/**
 * 断言数组长度
 */
export function assertLength<T>(arr: T[], length: number, message?: string): void {
    assert.strictEqual(arr.length, length, message ?? `Expected length ${length}, got ${arr.length}`);
}

/**
 * 断言抛出错误
 */
export async function assertThrows<T extends Error>(fn: () => Promise<void> | void, errorType?: new (...args: unknown[]) => T): Promise<T> {
    let thrown = false;
    let error: Error | null = null;

    try {
        await fn();
    } catch (e) {
        thrown = true;
        error = e instanceof Error ? e : new Error(String(e));
    }

    if (!thrown) {
        throw new assert.AssertionError({ message: "Expected function to throw an error" });
    }

    if (errorType && !(error instanceof errorType)) {
        throw new assert.AssertionError({
            message: `Expected error to be instance of ${errorType.name}, got ${error?.constructor.name}`,
        });
    }

    return error as T;
}

/**
 * 断言不抛出错误
 */
export async function assertNoThrow(fn: () => Promise<void> | void): Promise<void> {
    try {
        await fn();
    } catch (e) {
        throw new assert.AssertionError({
            message: `Expected function not to throw, but it threw: ${e instanceof Error ? e.message : String(e)}`,
        });
    }
}

/**
 * 断言大于
 */
export function assertGreater<T extends number | bigint>(actual: T, expected: T, message?: string): void {
    assertTrue(actual > expected, message ?? `Expected ${actual} > ${expected}`);
}

/**
 * 断言大于等于
 */
export function assertGreaterOrEqual<T extends number | bigint>(actual: T, expected: T, message?: string): void {
    assertTrue(actual >= expected, message ?? `Expected ${actual} >= ${expected}`);
}

/**
 * 断言小于
 */
export function assertLess<T extends number | bigint>(actual: T, expected: T, message?: string): void {
    assertTrue(actual < expected, message ?? `Expected ${actual} < ${expected}`);
}

/**
 * 断言小于等于
 */
export function assertLessOrEqual<T extends number | bigint>(actual: T, expected: T, message?: string): void {
    assertTrue(actual <= expected, message ?? `Expected ${actual} <= ${expected}`);
}

/**
 * 断言包含
 */
export function assertContains<T>(arr: T[], item: T, message?: string): void {
    assertTrue(arr.includes(item), message ?? `Expected array to contain ${item}`);
}

/**
 * 断言不包含
 */
export function assertNotContains<T>(arr: T[], item: T, message?: string): void {
    assertFalse(arr.includes(item), message ?? `Expected array not to contain ${item}`);
}

/**
 * 创建临时测试目录
 */
export function createTempDir(prefix: string = "lightdb-test-"): string {
    const tempDir = path.join(process.cwd(), "temp", `${prefix}${Date.now()}`);
    fs.mkdirSync(tempDir, { recursive: true });
    return tempDir;
}

/**
 * 删除临时测试目录
 */
export function removeTempDir(dir: string): void {
    if (fs.existsSync(dir)) {
        fs.rmSync(dir, { recursive: true, force: true });
    }
}

/**
 * 等待指定毫秒
 */
export function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * 生成随机字符串
 */
export function randomString(length: number = 10): string {
    const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let result = "";
    for (let i = 0; i < length; i++) {
        result += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return result;
}

/**
 * 生成随机整数
 */
export function randomInt(min: number, max: number): number {
    return Math.floor(Math.random() * (max - min + 1)) + min;
}

/**
 * 测试运行器
 */
export class TestRunner {
    private readonly suites: TestSuite[] = [];
    private readonly results: TestSuiteResult[] = [];

    public addSuite(suite: TestSuite): this {
        this.suites.push(suite);
        return this;
    }

    public async runAll(): Promise<void> {
        console.log("\n\x1b[35m========================================\x1b[0m");
        console.log("\x1b[35m         LightDB Test Runner\x1b[0m");
        console.log("\x1b[35m========================================\x1b[0m");

        const startTime = Date.now();
        let totalPassed = 0;
        let totalFailed = 0;

        for (const suite of this.suites) {
            const result = await suite.run();
            this.results.push(result);
            totalPassed += result.passed;
            totalFailed += result.failed;
        }

        const totalDuration = Date.now() - startTime;

        console.log("\n\x1b[35m========================================\x1b[0m");
        console.log("\x1b[35m            Test Summary\x1b[0m");
        console.log("\x1b[35m========================================\x1b[0m\n");

        for (const result of this.results) {
            const status = result.failed === 0 ? "\x1b[32m✓\x1b[0m" : "\x1b[31m✗\x1b[0m";
            console.log(`  ${status} ${result.suiteName}: ${result.passed}/${result.passed + result.failed} passed`);
        }

        console.log(`\n  Total: \x1b[32m${totalPassed} passed\x1b[0m, \x1b[31m${totalFailed} failed\x1b[0m (${totalDuration}ms)\n`);

        if (totalFailed > 0) {
            process.exit(1);
        }
    }
}

/**
 * 创建测试运行器
 */
export function createTestRunner(): TestRunner {
    return new TestRunner();
}
