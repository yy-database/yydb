# GitHub Actions / npm / Docker publish for YYDB

## Workflow split

| Workflow                                          | Trigger                              | Purpose                                    |
|---------------------------------------------------|--------------------------------------|--------------------------------------------|
| **CI** (`.github/workflows/ci.yml`)               | push/PR to `dev` / `main` / `master` | fmt, test, frontend build ? **no publish** |
| **Release** (`.github/workflows/release-npm.yml`) | push tag `v*.*.*` only               | npm + GitHub Release zips + GHCR docker    |

Release does **not** depend on CI. Tag `vX.Y.Z` ? package/image version `X.Y.Z`.

```text
git tag v0.1.0
git push origin v0.1.0
```

Jobs after `build-engine` run independently:

- `publish-npm` ? Trusted Publishing (OIDC)
- `github-release` ? create Release (if needed) and upload zips
- `docker` ? push `ghcr.io/<owner>/yydb` (`:vX.Y.Z`, `:X.Y.Z`, `:latest`)

Retry-safe:

- npm: version already on the registry ? skipped
- GitHub: release/asset already exists ? skipped
- Docker: same tags are overwritten (retry OK)

## Docker

Image: **`ghcr.io/yy-database/yydb`** (linux/amd64, from Release `yydb-linux-x64` artifact)

```bash
docker run --rm -p 7700:7700 -v yydb-data:/data \
  ghcr.io/yy-database/yydb:latest \
  serve /data/app.yydb --bind 0.0.0.0:7700 --insecure-bind
```

First publish may create a private GHCR package; set visibility under GitHub ? Packages if you want public pulls. No
extra secret: uses `GITHUB_TOKEN` + `packages: write`.

Details: [`docker/yydb/README.md`](../docker/yydb/README.md)

## Auth: Trusted Publishing (OIDC)

No long-lived `NPM_TOKEN` for publish. Docs: https://docs.npmjs.com/trusted-publishers/

npm org: **[`@yydb`](https://www.npmjs.com/settings/yydb/packages)**  
Packages: `@yydb/yydb`, `@yydb/yydb-client`, `@yydb/yydb-<os>-<arch>`

### Bootstrap `0.0.0` placeholders (once)

```text
npm login --auth-type=web
node scripts/publish-npm-placeholders.mjs
```

### Trusted Publisher fields (each package)

| Field                | Value             |
|----------------------|-------------------|
| Organization or user | `yy-database`     |
| Repository           | `yydb.rs`         |
| Workflow filename    | `release-npm.yml` |
| Environment name     | `NPM_PUBLISH`     |
| Allowed actions      | `npm publish`     |

- https://www.npmjs.com/package/@yydb/yydb-win32-x64/access
- https://www.npmjs.com/package/@yydb/yydb-linux-x64/access
- https://www.npmjs.com/package/@yydb/yydb-darwin-x64/access
- https://www.npmjs.com/package/@yydb/yydb-darwin-arm64/access
- https://www.npmjs.com/package/@yydb/yydb-client/access
- https://www.npmjs.com/package/@yydb/yydb/access

CLI helper (needs working web OTP): `node scripts/configure-trusted-publishers.mjs`

## Release job requirements

- `publish-npm`: `environment: NPM_PUBLISH`, `id-token: write`, Node ? 22.14, npm ? 11.5.1, no `NODE_AUTH_TOKEN`
- `github-release`: `contents: write`
- `docker`: `packages: write`
