---
name: yydb-release
description: >-
  Maintainer workflow for YYDB npm Trusted Publisher releases: Nifty
  placeholder/trust, semver git tags, and release-npm.yml OIDC publish. Load when
  publishing @yydb/*, cutting versions, or debugging OIDC/npm publish failures.
---

# YYDB release (maintainers)

npm publish for `@yydb/*` uses **GitHub Actions OIDC** — not local `npm publish` with tokens.

## When to load

- Cutting a semver release (`vX.Y.Z` tag)
- Configuring Trusted Publisher or Nifty `trust.npm`
- Debugging failed `release-npm.yml` or `ENEEDAUTH` on publish

## Do not load

- Day-to-day `cargo test` / `pnpm` dev (see check workflows)
- crates.io Rust publish (`cargo workspace publish` — separate path)
- User-facing install docs

## Workflows

| Workflow         | File                                     | Trigger                           | Purpose                                 |
|------------------|------------------------------------------|-----------------------------------|-----------------------------------------|
| Check Rust       | `.github/workflows/check-rust.yml`       | push/PR `dev` / `main` / `master` | `rustfmt`, `cargo test`, `napi` build   |
| Check TypeScript | `.github/workflows/check-typescript.yml` | push/PR `dev` / `main` / `master` | Nifty format, pnpm build/test           |
| Release          | `.github/workflows/release-npm.yml`      | push tag `v*.*.*`                 | npm OIDC (`@yydb/*` + platform `.node`) |

Release does **not** wait on CI green. Tag `vX.Y.Z` → npm version `X.Y.Z`.

```bash
git tag v0.0.3
git push origin v0.0.3
```

Retry-safe: already-published npm versions are skipped.

## `nifty.config.ts`

| Key                     | Purpose                                |
|-------------------------|----------------------------------------|
| `trust.npm.repo`        | `yy-database/yydb`                     |
| `trust.npm.file`        | `release-npm.yml`                      |
| `trust.npm.environment` | `NPM_PUBLISH`                          |
| `publish.packages`      | Every `@yydb/*` name for `nifty trust` |

Prefer `trust.npm` over `NIFTY_TRUST_*` env vars. `nifty trust` needs `NPM_TOKEN` or `npm login`.

## Placeholder bootstrap

```bash
pnpm install
pnpm placeholder          # dry-run
pnpm placeholder:publish  # nifty publish --placeholder
```

Uses `NPM_TOTP_SECRET` in `.env.placeholder.local`. Cache: `.cache/npm-placeholder.json`.

## Trusted Publisher

```bash
pnpm placeholder:trust
```

| Field                | Value             |
|----------------------|-------------------|
| Organization or user | `yy-database`     |
| Repository           | `yydb`            |
| Workflow filename    | `release-npm.yml` |
| Environment name     | `NPM_PUBLISH`     |

CI `publish-npm`: `environment: NPM_PUBLISH`, `id-token: write`, Node ≥ 22.14, npm ≥ 11.5.1, **no** `NODE_AUTH_TOKEN`.

## Published packages

`@yydb/yydb`, `@yydb/yydb-client`, `@yydb/yydb-win32-x64`, `@yydb/yydb-linux-x64`, `@yydb/yydb-darwin-x64`,
`@yydb/yydb-darwin-arm64`

Tag release runs `scripts/prepare-npm-publish.mjs` then `scripts/publish-npm-packages.mjs` (OIDC).

## Example prompt

```text
Load yydb-release. release-npm publish job failed with ENEEDAUTH — walk through Trusted Publisher and npmrc checks.
```
