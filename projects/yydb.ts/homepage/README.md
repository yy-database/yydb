# `@yydb/yydb-homepage`

YYDB product homepage and user guides (Simplified Chinese and English). Built with [`@vmz/vmz`](https://www.npmjs.com/package/@vmz/vmz) `0.3.8`.

## Commands

```bash
pnpm --filter @yydb/yydb-homepage dev
pnpm --filter @yydb/yydb-homepage build
pnpm --filter @yydb/yydb-homepage document:check
```

## Routes

| Path | Purpose |
|------|---------|
| `/welcome` | Full-screen welcome |
| `/` | Product landing |
| `/install` | Install and platform guide |
| `/d/` | Integrated documentation (`/d/zh-hans/`, `/d/en/`) |
| `/wire` | Local `@yydb/yydb-client` frame encode/decode demo |

## Related packages

| Package | Use |
|---------|-----|
| [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb) | Node.js file API |
| [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) | Browser and wire client |
| [`@yydb/yydb-webui`](https://www.npmjs.com/package/@yydb/yydb-webui) | Local database explorer |
| [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills) | Agent skills for integrators |

[YYDB overview](https://github.com/yy-database/yydb) · [Rust API](https://docs.rs/yydb)
