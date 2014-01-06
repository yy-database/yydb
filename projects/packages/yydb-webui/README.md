# `@yydb/yydb-webui`

**A local browser UI for a running YYDB server.**

Use it to inspect server info and a VOS schema, or try a few key/value reads and writes while developing an application.
It is a local tool, not a hosted database console.

## Quick start

Start an engine in one terminal:

```text
yydb serve ./app.yydb --bind 127.0.0.1:7700
```

Run the UI from the repository in another:

```bash
pnpm install
pnpm --filter @yydb/yydb-client build
pnpm --filter @yydb/yydb-webui dev
```

Open the local URL printed by Vite (usually
`http://127.0.0.1:5173`) and connect to the default endpoint
`ws://127.0.0.1:7700/wire`.

The UI can:

- show server information;
- load or ensure a VOS schema and version;
- read and write string key/value records.

The UI uses [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client)
over the WebSocket `/wire` endpoint.

## Build a static bundle

```bash
pnpm --filter @yydb/yydb-webui build
pnpm --filter @yydb/yydb-webui preview
```

The output is written to `dist/` and can be hosted as static assets next to a private YYDB endpoint.

## Security

The reference server has no authentication. Keep the UI and its endpoint on your machine or on a trusted private
network; do not expose `yydb serve` to the public internet.

## Links

- [Node file API](https://www.npmjs.com/package/@yydb/yydb)
- [YYDB user guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
- [Docker image](https://github.com/yy-database/yydb.rs/tree/dev/docker/yydb)
- [License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
