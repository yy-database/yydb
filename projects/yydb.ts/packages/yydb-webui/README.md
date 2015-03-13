# `@yydb/yydb-webui`

Local browser UI for a running YYDB server ù inspect server info and VOS schema, try key/value reads and writes while developing. Built on [VMZ](https://github.com/doki-land/vmz-framework) and `@vmz/ui` console components.

## Quick start

Open a database in one step:

```text
yydb webui ./app.yydb
```

This starts `yydb serve`, serves the WebUI on `http://127.0.0.1:8780`, and opens your browser with the wire endpoint pre-filled. The page connects automatically when launched from the CLI.

## Manual workflow

```text
yydb serve ./app.yydb --bind 127.0.0.1:7700
yydb webui --endpoint ws://127.0.0.1:7700/wire --no-open
```

Or develop the UI package directly:

```text
pnpm --filter @yydb/yydb-webui dev
```

`pnpm --filter @yydb/yydb build` compiles the CLI and copies `dist/cdn` into `@yydb/yydb/dist/webui` so `yydb webui` works from a single published package.

## Capabilities

The UI uses [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) over WebSocket `/wire`:

- server information;
- load or ensure a VOS schema and version;
- read and write string key/value records.

The reference server has no authentication. Keep the UI and endpoint on your machine or a trusted private network.

[Node file API](https://www.npmjs.com/package/@yydb/yydb) ù [YYDB overview](https://github.com/yy-database/yydb)
