# `ghcr.io/yy-database/yydb`

**The official container image for the YYDB CLI and local wire server.**

The image ships the TypeScript `yydb` CLI (`node dist/cli.js`) plus the `linux/x64` `yydb-napi` binding. It is currently built for
`linux/amd64`.

[![GHCR](https://img.shields.io/badge/ghcr.io-yy--database%2Fyydb-blue)](https://github.com/yy-database/yydb.rs/pkgs/container/yydb)
[![Release](https://img.shields.io/github/v/release/yy-database/yydb.rs?include_prereleases&sort=semver)](https://github.com/yy-database/yydb.rs/releases)

## Run a server

Mount a volume at `/data` so the database survives container replacement:

```bash
docker run --rm \
  -p 7700:7700 \
  -v yydb-data:/data \
  ghcr.io/yy-database/yydb:latest \
  serve /data/app.yydb --bind 0.0.0.0:7700 --insecure-bind
```

The WebSocket endpoint is `ws://127.0.0.1:7700/wire` from the host. Connect with [
`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) or
the [local WebUI](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/yydb-webui).

## Initialize or inspect a file

```bash
docker run --rm -v yydb-data:/data \
  ghcr.io/yy-database/yydb:latest \
  init /data/app.yydb

docker run --rm -v yydb-data:/data \
  ghcr.io/yy-database/yydb:latest \
  info /data/app.yydb
```

## Image tags

| Tag                               | Use                                                |
|-----------------------------------|----------------------------------------------------|
| `ghcr.io/yy-database/yydb:0.1.0`  | Reproducible release (recommended for deployments) |
| `ghcr.io/yy-database/yydb:v0.1.0` | Release tag with the Git tag prefix                |
| `ghcr.io/yy-database/yydb:latest` | Most recent release                                |

## Network and access

The reference `serve` endpoint has no authentication. The container example uses `0.0.0.0` so Docker port publishing can
reach it; place it on a trusted private network or add access control at the surrounding proxy/firewall. For a process
that does not need container networking, prefer a loopback bind:

```text
yydb serve app.yydb --bind 127.0.0.1:7700
```

## Links

- [YYDB overview](https://github.com/yy-database/yydb.rs)
- [User guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
- [Release archives](https://github.com/yy-database/yydb.rs/releases)
- [License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
