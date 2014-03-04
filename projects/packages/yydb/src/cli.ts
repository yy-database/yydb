#!/usr/bin/env node
import { runCli } from "./cli/cli.js";

const code = await runCli(process.argv);
if (code !== 0) {
    process.exitCode = code;
}
