---
name: yydb-bytes-storage
description: >-
    Historical per-object CAS bytes layout draft for YYDB. Not current `.yydb` contract.
    Load only when comparing legacy CAS experiments or YYDS bytes alignment.
---

# Historical YYDB bytes storage draft

> **Not current contract.** Describes an earlier per-object CAS experiment. Current `.yydb` is a single file with
> `-wal`/`-shm`. Only `.yydx` may use sibling `<app-name>-objects/` with `.blob` segments. Do not create
> `<app-name>.yydb.objects/` from this note.

## Layout (historical)

```text
<db>.objects/
└── objects/
    └── ab/
        └── abcd…ef.bytes
```

Default BLAKE3 hash-2 paths. Rows hold `ObjectRef`; large files use chunk manifests.

## Policy

| Approach                     | Policy                               |
|------------------------------|--------------------------------------|
| Inline in `.yydb`            | Allowed for tiny values; not default |
| CAS `objects/hash-2/*.bytes` | Recommended                          |

Soft threshold: `INLINE_BYTES_MAX` (4 KiB). Large files: chunked manifest + range reads.

Vectors prefer CAS; ANN graphs are `ObjectKind::AnnSegment` objects.

Hot/cold tiering is runtime cache on the same CAS (`pin_object`, `evict_object`).

Full backup needs `.yydb` (+ wal/shm) and `<db>.objects/` tree.

YYDS multi-file layout should align ObjectRef/chunk semantics with this model where applicable.
