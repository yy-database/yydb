---
name: yydb-release
description: >-
    YYDB npm Trusted Publisher release: Nifty placeholder/trust, tag `vX.Y.Z`, CI workflows.
    Load when publishing `@yydb/*`, cutting versions, or OIDC on `yy-database/yydb`.
---

# YYDB release

## Workflows

| Workflow         | File                                     | Trigger                           | Purpose                               |
|------------------|------------------------------------------|-----------------------------------|---------------------------------------|
| Check Rust       | `.github/workflows/check-rust.yml`       | push/PR `dev` / `main` / `master` | `rustfmt`, `cargo test`, `napi`       |
| Check TypeScript | `.github/workflows/check-typescript.yml` | push/PR `dev` / `main` / `master` | Nifty format, pnpm build/test         |
| Release          | `.github/workflows/release-npm.yml`      | push tag `v*.*.*`                 | npm OIDC + GitHub Release engine zips |

Release does **not** depend on CI. Tag `vX.Y.Z` → npm version `X.Y.Z`.

```text
git tag v0.0.2
git push origin v0.0.2
```

Jobs after `build-engine`: `publish-npm` (OIDC) and `github-release` (zips). Retry-safe — existing npm versions /
release assets are skipped.

## `nifty.config.ts`

Set `publish.packages` to every `@yydb/*` name for **`nifty trust`**.

## Placeholder bootstrap

```bash
pnpm install
pnpm placeholder          # dry-run
pnpm placeholder:publish  # nifty publish --placeholder
```

Uses **`NPM_TOTP_SECRET`** in **`.env.placeholder.local`** (not `npm login`). Cache: `.cache/npm-placeholder.json`.

## Trusted Publisher

```bash
$env:NIFTY_TRUST_FILE = "release-npm.yml"
pnpm placeholder:trust
```

| Field                | Value             |
|----------------------|-------------------|
| Organization or user | `yy-database`     |
| Repository           | `yydb`            |
| Workflow filename    | `release-npm.yml` |
| Environment name     | `NPM_PUBLISH`     |

CI `publish-npm`: `environment: NPM_PUBLISH`, `id-token: write`, Node ≥ 22.14, npm ≥ 11.5.1, **no** `NODE_AUTH_TOKEN`.

## Packages

`@yydb/yydb`, `@yydb/yydb-client`, `@yydb/yydb-win32-x64`, `@yydb/yydb-linux-x64`, `@yydb/yydb-darwin-x64`,
`@yydb/yydb-darwin-arm64`

Tag release uses `scripts/prepare-npm-publish.mjs` + `scripts/publish-npm-packages.mjs` (OIDC).
