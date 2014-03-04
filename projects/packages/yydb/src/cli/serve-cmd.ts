import type { Cli, ParsedOptions } from "@vmz/commander";

import { loadNative } from "../native.js";
import { flag, requiredPositional, str } from "./options.js";

export function registerServeCommand(cli: Cli): void {
    cli.command("serve", "cli.cmd.serve")
        .option("--bind <host:port>", "cli.opt.bind")
        .option("--insecure-bind", "cli.opt.insecure-bind")
        .action((options) => cmdServe(options));
}

export function cmdServe(options: ParsedOptions): number {
    const dbPath = requiredPositional(options, "serve");
    const bind = str(options, "bind") ?? "127.0.0.1:7700";
    loadNative().serve(dbPath, bind, flag(options, "insecure-bind"));
    return 0;
}
