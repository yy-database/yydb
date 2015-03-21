# Serve and CLI

The `yydb` CLI ships with `@yydb/yydb`. Run it through `npx` with no global install.

## Commands

| Command | Purpose |
|---------|---------|
| `yydb version` | Print the engine version |
| `yydb init ./app.yydb --schema-file schema.vos` | Create a file, optionally publishing a schema |
| `yydb info ./app.yydb --doctor` | Print path, schema, record count and journal mode; `--doctor` adds a consistency report |
| `yydb verify ./app.yydb` | Read-only consistency check |
| `yydb serve ./app.yydb --bind 127.0.0.1:7700` | Start a wire protocol endpoint |
| `yydb webui ./app.yydb` | Start the local WebUI and open a browser |

## verify exit codes

| Exit code | Meaning |
|-----------|---------|
| `0` | No issues found |
| `13` | Warnings only |
| `1` | Errors present |

CI can use these to distinguish "must stop" from "needs human review".

## serve options

| Option | Default | Notes |
|--------|---------|-------|
| `--bind <host:port>` | `127.0.0.1:7700` | Listen address. Port and path are up to your deployment |
| `--insecure-bind` | Off | Allow binding a non-loopback address |

The service accepts both TCP and WebSocket at `/wire`.

## WebUI

```bash
npx @yydb/yydb webui ./app.yydb
```

| Option | Default | Notes |
|--------|---------|-------|
| `--bind <host:port>` | `127.0.0.1:7700` | Bind address of the underlying serve |
| `--port <number>` | `8780` | WebUI page port |
| `--endpoint <url>` | Derived from bind | Connect to an existing serve instead of starting one |
| `--no-open` | Off | Do not open a browser automatically |

The WebUI is a local debugging interface, not a multi-user console.

## Security model

> Warning: the reference `yydb serve` has no login, tenancy, roles, authorization or audit. Any client that can reach the endpoint can perform every operation the protocol allows.

Therefore:

1. Bind `127.0.0.1` by default. Do not switch to `0.0.0.0` for convenience.
2. For cross-machine access, restrict the source with an SSH tunnel, a VPN or a reverse proxy instead of relying on `serve` itself.
3. Use YYDS when you need identity, permissions, audit and quotas.
4. Operating-system file permissions are the real data boundary. Anyone who can read the `.yydb` file does not need the service at all.

## When serve is the right choice

- A browser or another process needs access to data on the Node side.
- You need a single-writer model to avoid multi-process write-lock conflicts.
- You need to expose one controlled endpoint locally.

If the application already lives in the Node process, `Database.open` is simpler and has no network attack surface.
