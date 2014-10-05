/**
 * 浏览器端测试驱动
 *
 * 用本地 HTTP 服务按需把 TypeScript 转译成 ESM 提供给浏览器，
 * 再用本机已安装的 Chrome 运行 `tests/browser` 下的一致性套件与多标签页场景。
 * 不下载托管浏览器，也不引入打包器。
 */

import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import type { AddressInfo } from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import type { Browser, BrowserContext, Page } from 'playwright-core';
import ts from 'typescript';

/** 当前目录：`tests/browser` */
const HERE = path.dirname(fileURLToPath(import.meta.url));

/** 包根目录 */
const PACKAGE_ROOT = path.resolve(HERE, '../..');

/** 浏览器启动通道，可用环境变量覆盖 */
const BROWSER_CHANNEL = process.env.LIGHT_DB_BROWSER_CHANNEL ?? 'chrome';

/** 单框架类型 */
type CaseResult = { name: string; ok: boolean; error?: string };

/** 打开尝试结果 */
type OpenAttempt = { ok: boolean; code?: string; retryable?: boolean; value?: string | null };

/** 页面级结果作用域 */
interface PageScope {
    __lightDbDone: boolean;
    __lightDbResults: CaseResult[];
}

/**
 * 启动静态服务
 */
async function startServer(): Promise<{ origin: string; close: () => Promise<void> }> {
    const server = createServer(async (request, response) => {
        const url = new URL(request.url ?? '/', 'http://127.0.0.1');
        try {
            const resolved = await resolveAsset(url.pathname);
            if (!resolved) {
                response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
                response.end('not found');
                return;
            }
            if (resolved.kind === 'redirect') {
                response.writeHead(302, { location: resolved.location });
                response.end();
                return;
            }
            response.writeHead(200, { 'content-type': resolved.type, 'cache-control': 'no-store' });
            response.end(resolved.body);
        } catch (error) {
            response.writeHead(500, { 'content-type': 'text/plain; charset=utf-8' });
            response.end(error instanceof Error ? error.message : String(error));
        }
    });

    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    const address = server.address() as AddressInfo;

    return {
        origin: `http://127.0.0.1:${address.port}`,
        close: () =>
            new Promise<void>((resolve, reject) => {
                server.close((error) => (error ? reject(error) : resolve()));
            }),
    };
}

/** 响应内容 */
type ResolvedAsset = { kind: 'asset'; body: string | Buffer; type: string } | { kind: 'redirect'; location: string };

/**
 * 解析请求路径到响应内容
 *
 * 浏览器按 URL 解析相对模块说明符，因此模块 URL 必须以 `.js` 结尾；
 * 无扩展名的说明符先重定向到规范 URL，避免 `index` 模块被当成文件而错位一级。
 * @param pathname 请求路径
 */
async function resolveAsset(pathname: string): Promise<ResolvedAsset | null> {
    if (pathname === '/' || pathname === '/index.html') {
        const asset = await readStatic(path.join(HERE, 'index.html'));
        return asset ? { kind: 'asset', ...asset } : null;
    }
    if (pathname === '/favicon.ico') {
        return { kind: 'asset', body: '', type: 'image/x-icon' };
    }

    const decoded = decodeURIComponent(pathname);
    const candidate = path.resolve(PACKAGE_ROOT, `.${decoded}`);
    if (!candidate.startsWith(PACKAGE_ROOT)) {
        return null;
    }

    if (!candidate.endsWith('.js')) {
        const canonical = await canonicalModuleUrl(candidate, decoded);
        return canonical ? { kind: 'redirect', location: canonical } : null;
    }

    const sourceFile = `${candidate.slice(0, -3)}.ts`;
    const source = await tryRead(sourceFile);
    if (source !== null) {
        return { kind: 'asset', body: transpile(source, sourceFile), type: 'text/javascript; charset=utf-8' };
    }

    const asset = await readStatic(candidate);
    return asset ? { kind: 'asset', ...asset } : null;
}

/**
 * 计算模块的规范 URL
 * @param candidate 文件系统路径
 * @param pathname 原始请求路径
 */
async function canonicalModuleUrl(candidate: string, pathname: string): Promise<string | null> {
    if ((await tryRead(`${candidate}.ts`)) !== null) {
        return `${pathname}.js`;
    }
    if ((await tryRead(path.join(candidate, 'index.ts'))) !== null) {
        return `${pathname.replace(/\/$/, '')}/index.js`;
    }
    if ((await tryRead(path.join(candidate, 'index.js'))) !== null) {
        return `${pathname.replace(/\/$/, '')}/index.js`;
    }
    if ((await tryRead(`${candidate}.js`)) !== null) {
        return `${pathname}.js`;
    }
    return null;
}

/**
 * 读取静态文件
 * @param file 绝对路径
 */
async function readStatic(file: string): Promise<{ body: string | Buffer; type: string } | null> {
    const body = await tryRead(file);
    if (body === null) {
        return null;
    }
    return { body, type: mimeType(file) };
}

/**
 * 尝试读取文件，不存在时返回 null
 * @param file 绝对路径
 */
async function tryRead(file: string): Promise<Buffer | null> {
    try {
        return await readFile(file);
    } catch {
        return null;
    }
}

/**
 * 把 TypeScript 转译为浏览器可执行的 ESM
 * @param source 源码
 * @param fileName 文件名，用于诊断
 */
function transpile(source: Buffer, fileName: string): string {
    return ts.transpileModule(source.toString('utf8'), {
        fileName,
        compilerOptions: {
            target: ts.ScriptTarget.ES2022,
            module: ts.ModuleKind.ESNext,
            moduleResolution: ts.ModuleResolutionKind.Bundler,
            useDefineForClassFields: true,
        },
    }).outputText;
}

/**
 * 推断 MIME
 * @param file 文件名
 */
function mimeType(file: string): string {
    switch (path.extname(file)) {
        case '.html':
            return 'text/html; charset=utf-8';
        case '.js':
            return 'text/javascript; charset=utf-8';
        case '.json':
            return 'application/json; charset=utf-8';
        default:
            return 'text/plain; charset=utf-8';
    }
}

/**
 * 运行单页一致性套件
 * @param page 页面
 * @param origin 服务源
 */
async function runPageSuite(page: Page, origin: string): Promise<CaseResult[]> {
    page.on('console', (message) => {
        console.log(`  [page:${message.type()}] ${message.text()}`);
    });
    page.on('pageerror', (error) => {
        console.log(`  [page:error] ${error.message}`);
    });
    page.on('requestfailed', (request) => {
        console.log(`  [page:request] ${request.url()} ${request.failure()?.errorText ?? ''}`);
    });

    await page.goto(`${origin}/`, { waitUntil: 'load' });
    await page.waitForFunction(() => (globalThis as unknown as PageScope).__lightDbDone === true, undefined, { timeout: 60_000 });
    return page.evaluate(() => (globalThis as unknown as PageScope).__lightDbResults);
}

/**
 * 运行多标签页场景
 * @param context 浏览器上下文
 * @param origin 服务源
 */
async function runMultitab(context: BrowserContext, origin: string): Promise<CaseResult[]> {
    const results: CaseResult[] = [];
    const name = `test-multitab-${Date.now().toString(36)}`;

    const tabA = await context.newPage();
    const tabB = await context.newPage();
    await Promise.all([tabA.goto(`${origin}/`, { waitUntil: 'load' }), tabB.goto(`${origin}/`, { waitUntil: 'load' })]);

    try {
        const held = await openAndHold(tabA, name, 'shared', 'v1');
        push(results, '标签页 A 取得写租约', held.ok, held.code);

        await listenDatabase(tabB, name);

        const blocked = await tryOpenAndRead(tabB, name, 'shared');
        push(results, '标签页 B 打开时得到 lease.unavailable', !blocked.ok && blocked.code === 'yydb.lease.unavailable', blocked.code);
        push(results, '写租约冲突标记为可重试', blocked.retryable === true, undefined);

        const rewritten = await writeHeld(tabA, 'shared', 'v2');
        push(results, '标签页 A 完成第二次提交', rewritten.ok, rewritten.code);

        await tabB.waitForFunction(
            () => ((globalThis as unknown as { __yydbEvents?: string[] }).__yydbEvents ?? []).includes('commit'),
            undefined,
            { timeout: 10_000 },
        );
        const commitEvents = await collectedEvents(tabB);
        push(results, '标签页 B 收到 commit 通知', commitEvents.includes('commit'), commitEvents.join(','));

        await closeHeld(tabA);
        await tabB.waitForFunction(
            () => ((globalThis as unknown as { __yydbEvents?: string[] }).__yydbEvents ?? []).includes('close'),
            undefined,
            { timeout: 10_000 },
        );
        push(results, '标签页 B 收到 close 通知', (await collectedEvents(tabB)).includes('close'), undefined);

        const reopened = await tryOpenAndRead(tabB, name, 'shared');
        push(results, '租约释放后标签页 B 可以打开', reopened.ok, reopened.code);
        push(results, '标签页 B 读到最后一次提交', reopened.value === 'v2', String(reopened.value));
    } finally {
        await tabA.close();
        await tabB.close();
    }

    return results;
}

/**
 * 在标签页里打开并保持数据库
 * @param page 页面
 * @param name 数据库名
 * @param key 键
 * @param value 值
 */
function openAndHold(page: Page, name: string, key: string, value: string): Promise<OpenAttempt> {
    return page.evaluate(
        async (input) => {
            const module = await import('/tests/browser/multitab.js');
            return module.openAndHold(input.name, input.key, input.value);
        },
        { name, key, value },
    ) as Promise<OpenAttempt>;
}

/**
 * 在标签页里监听数据库事件
 * @param page 页面
 * @param name 数据库名
 */
async function listenDatabase(page: Page, name: string): Promise<void> {
    await page.evaluate(async (input) => {
        const module = await import('/tests/browser/multitab.js');
        module.listenDatabase(input);
    }, name);
}

/**
 * 尝试打开并读取
 * @param page 页面
 * @param name 数据库名
 * @param key 键
 */
function tryOpenAndRead(page: Page, name: string, key: string): Promise<OpenAttempt> {
    return page.evaluate(
        async (input) => {
            const module = await import('/tests/browser/multitab.js');
            return module.tryOpenAndRead(input.name, input.key);
        },
        { name, key },
    ) as Promise<OpenAttempt>;
}

/**
 * 在持有的数据库上再写一次
 * @param page 页面
 * @param key 键
 * @param value 值
 */
function writeHeld(page: Page, key: string, value: string): Promise<OpenAttempt> {
    return page.evaluate(
        async (input) => {
            const module = await import('/tests/browser/multitab.js');
            return module.writeHeld(input.key, input.value);
        },
        { key, value },
    ) as Promise<OpenAttempt>;
}

/**
 * 关闭标签页持有的数据库
 * @param page 页面
 */
async function closeHeld(page: Page): Promise<void> {
    await page.evaluate(async () => {
        const module = await import('/tests/browser/multitab.js');
        await module.closeHeld();
    });
}

/**
 * 读取已收集事件
 * @param page 页面
 */
async function collectedEvents(page: Page): Promise<string[]> {
    return page.evaluate(async () => {
        const module = await import('/tests/browser/multitab.js');
        return module.collectedEvents();
    });
}

/**
 * 记录断言结果
 * @param results 结果
 * @param name 用例名
 * @param ok 是否通过
 * @param detail 细节
 */
function push(results: CaseResult[], name: string, ok: boolean, detail: string | undefined): void {
    if (ok || detail === undefined) {
        results.push({ name, ok });
        return;
    }
    results.push({ name, ok, error: detail });
}

/**
 * 打印并汇总
 * @param results 全部结果
 */
function report(results: CaseResult[]): void {
    console.log('\n========================================');
    console.log('        Light DB Browser Suite');
    console.log('========================================\n');

    let failed = 0;
    for (const result of results) {
        if (result.ok) {
            console.log(`  ✓ ${result.name}`);
            continue;
        }
        failed += 1;
        console.log(`  ✗ ${result.name}`);
        if (result.error) {
            console.log(`      ${result.error}`);
        }
    }

    console.log(`\n  Total: ${results.length - failed} passed, ${failed} failed\n`);
    if (failed > 0) {
        process.exitCode = 1;
    }
}

/**
 * 驱动入口
 */
async function main(): Promise<void> {
    const server = await startServer();
    let browser: Browser | null = null;
    const results: CaseResult[] = [];

    try {
        browser = await chromium.launch({ channel: BROWSER_CHANNEL, headless: true });
        const context = await browser.newContext();
        const page = await context.newPage();
        results.push(...(await runPageSuite(page, server.origin)));
        await page.close();
        results.push(...(await runMultitab(context, server.origin)));
        await context.close();
    } finally {
        if (browser) {
            await browser.close();
        }
        await server.close();
    }

    report(results);
}

main().catch((error: unknown) => {
    console.error(error);
    process.exitCode = 1;
});
