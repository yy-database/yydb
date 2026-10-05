# `@yydb/yydb-webui`

Local browser UI for a running YYDB server — inspect server info and VOS schema, try key/value reads and writes while
developing. Not a hosted database console.

## Usage

Start an engine:

```text
yydb serve ./app.yydb --bind 127.0.0.1:7700
```

Install `@yydb/yydb-webui` from npm, run its dev or preview script, and connect to `ws://127.0.0.1:7700/wire`.

The UI uses [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) over WebSocket `/wire` and supports:

- server information;
- load or ensure a VOS schema and version;
- read and write string key/value records.

## Example session

1. Run `yydb serve` on a `.yydb` file (see above).
2. Open the WebUI in your browser.
3. Point the connection field at `ws://127.0.0.1:7700/wire`.
4. Use **Info** and **Schema** panels, then exercise **Get** / **Put** on string keys.

The reference server has no authentication. Keep the UI and endpoint on your machine or a trusted private network.

[Node file API](https://www.npmjs.com/package/@yydb/yydb) · [YYDB overview](https://github.com/yy-database/yydb.rs) · [User guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
