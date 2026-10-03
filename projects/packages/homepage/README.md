# YYDB user documentation

This package contains the YYDB product homepage and the Chinese user guides. It is the documentation surface for people
evaluating or using YYDB; the database engine and client packages live alongside it in this repository.

## User guides

- [中文文档首页](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
- [嵌入与
  `serve`](https://github.com/yy-database/yydb.rs/blob/dev/projects/packages/homepage/documentation/zh-hans/guide/embed-and-serve.md)
- [Serve 安全边界](https://github.com/yy-database/yydb.rs/blob/dev/projects/packages/homepage/documentation/zh-hans/guide/serve-security.md)
- [线协议概览](https://github.com/yy-database/yydb.rs/blob/dev/projects/packages/homepage/documentation/zh-hans/guide/wire-protocol.md)

## Run the docs site locally

```bash
pnpm install
pnpm --filter @yydb/yydb-client build
pnpm --filter @yydb/yydb-homepage dev
```

Create a production build with:

```bash
pnpm --filter @yydb/yydb-homepage build
```

Integrator Agent Skills ship in [`@yydb/yydb-skills`](../yydb-skills) (`npx skills add @yydb/yydb-skills --list`).

## Related packages

| Package                                                                                            | Use                     |
|----------------------------------------------------------------------------------------------------|-------------------------|
| [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)                                           | Node.js file API        |
| [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client)                             | Browser and wire client |
| [`@yydb/yydb-webui`](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/yydb-webui) | Local database explorer |
