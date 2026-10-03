# `@yydb/yydb-skills`

Agent Skills for integrators: wire protocol, VOS codegen, and historical bytes layout notes. Install a skill when you need an agent to implement or review YY-family clients, servers, or schema tooling.

## Install

```bash
npx skills add @yydb/yydb-skills --skill yydb-serve-protocol -y
```

List skills or install from the monorepo path before npm publish:

```bash
npx skills add @yydb/yydb-skills --list
npx skills add ./projects/packages/yydb-skills --skill yydb-serve-protocol -y
```

Requires Node.js 18+ for the installer. Skills are docs-only and do not replace `@yydb/yydb` or `@yydb/yydb-client`.

## Skills

| Skill                 | Use when                                                     |
|-----------------------|--------------------------------------------------------------|
| `yydb-serve-protocol` | TCP/WebSocket wire frames, `yydb serve`, `@yydb/yydb-client` |
| `yydb-generator`      | `yydb generate`, `@yydb/yydb/generator`, VOS → TypeScript    |
| `yydb-bytes-storage`  | Legacy per-object CAS layout (not current `.yydb` contract)  |

Human-readable guides also live in [`@yydb/yydb-homepage`](../homepage/documentation/zh-hans/).
