/**
 * LightDB 测试入口
 *
 * 产品代码是浏览器原生的，测试运行器仍用 Node 执行纯 TypeScript 逻辑；
 * OPFS 与多标签页合同由浏览器端测试单独覆盖。
 */

import { pathToFileURL } from "node:url";
import { createTestRunner } from "./test-utils";
import { createAdapterTests } from "./adapters.test";
import { createCodecTests } from "./codec.test";
import { createDatabaseTests } from "./database.test";

/**
 * 运行单元测试
 */
async function runUnitTests(): Promise<void> {
    const runner = createTestRunner();
    runner.addSuite(createCodecTests());
    runner.addSuite(createAdapterTests());
    runner.addSuite(createDatabaseTests());
    await runner.runAll();
}

export { runUnitTests };

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
    runUnitTests().catch((error: unknown) => {
        console.error(error);
        process.exitCode = 1;
    });
}
