# AGENTS

YYDB is a local database that use [vos](https://github.com/voml/vos-language) syntax.

## Maintainer skills (`.agents/skills/`)

For repository agents only. Do not link these from user-facing READMEs or homepage docs.

| Skill        | Path                                   | When to load                                               |
|--------------|----------------------------------------|------------------------------------------------------------|
| YYDB release | `.agents/skills/yydb-release/SKILL.md` | npm publish, placeholders, Trusted Publisher, tag `v*.*.*` |

## Downstream skills (`@yydb/yydb-skills`)

User- and integrator-facing Agent Skills live in `projects/yydb.ts/packages/yydb-skills/`.

Human guides stay in `projects/yydb.ts/homepage/documentation/zh-hans/`.

Root `documentation/` is developer-facing design guidance: invariants, layering, encoding and compatibility, engineering standards, UX/DX rules. It is not user usage docs and not a roadmap. Do not create `/docs`.

## Quick commands

```bash
pnpm install
pnpm test
pnpm fmt:check
pnpm placeholder:publish   # nifty publish --placeholder
pnpm placeholder:trust     # nifty trust (reads trust.npm from nifty.config.ts)
```
