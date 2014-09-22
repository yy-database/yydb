/**
 * LightDB 测试入口
 * 运行所有测试套件
 */

import { pathToFileURL } from "node:url";
import { createTestRunner } from "./test-utils";
import { getStorageEngineTests } from "./storage-engine.test";
import { getIndexEngineTests } from "./index-engine.test";
import { getCollectionTests } from "./collection.test";
import { getAPITests } from "./api.test";
import { getBenchmarkTests } from "./benchmark.test";

/**
 * 运行所有测试
 */
async function runAllTests(): Promise<void> {
    const runner = createTestRunner();

    const storageTests = getStorageEngineTests();
    for (const suite of storageTests) {
        runner.addSuite(suite);
    }

    const indexTests = getIndexEngineTests();
    for (const suite of indexTests) {
        runner.addSuite(suite);
    }

    const collectionTests = getCollectionTests();
    for (const suite of collectionTests) {
        runner.addSuite(suite);
    }

    const apiTests = getAPITests();
    for (const suite of apiTests) {
        runner.addSuite(suite);
    }

    const benchmarkTests = getBenchmarkTests();
    for (const suite of benchmarkTests) {
        runner.addSuite(suite);
    }

    await runner.runAll();
}

/**
 * 仅运行单元测试（排除性能测试）
 */
async function runUnitTests(): Promise<void> {
    const runner = createTestRunner();

    const storageTests = getStorageEngineTests();
    for (const suite of storageTests) {
        runner.addSuite(suite);
    }

    const indexTests = getIndexEngineTests();
    for (const suite of indexTests) {
        runner.addSuite(suite);
    }

    const collectionTests = getCollectionTests();
    for (const suite of collectionTests) {
        runner.addSuite(suite);
    }

    const apiTests = getAPITests();
    for (const suite of apiTests) {
        runner.addSuite(suite);
    }

    await runner.runAll();
}

/**
 * 仅运行性能基准测试
 */
async function runBenchmarkTests(): Promise<void> {
    const runner = createTestRunner();

    const benchmarkTests = getBenchmarkTests();
    for (const suite of benchmarkTests) {
        runner.addSuite(suite);
    }

    await runner.runAll();
}

/**
 * 导出测试函数
 */
export { runAllTests, runUnitTests, runBenchmarkTests };

/**
 * 主入口
 */
if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
    const args = process.argv.slice(2);
    const testType = args[0] || "all";

    switch (testType) {
        case "unit":
            runUnitTests().catch(console.error);
            break;
        case "benchmark":
            runBenchmarkTests().catch(console.error);
            break;
        case "all":
        default:
            runAllTests().catch(console.error);
            break;
    }
}
