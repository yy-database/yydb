---
name: yydb-bytes-storage
description: >-
  Historical per-object CAS bytes layout draft for YYDB. Not the current .yydb
  file contract. Use only when comparing legacy CAS experiments or YYDS bytes
  alignment — never for new .yydb storage design.
---

# Historical YYDB bytes storage draft

> **Not current contract.** Describes an earlier per-object CAS experiment. Current `.yydb` is a single file with
> `-wal`/`-shm`. Only `.yydx` may use sibling `<app-name>-objects/` with `.blob` segments. Do not create
> `<app-name>.yydb.objects/` from this note.

## When to load

- Auditing old docs or branches that reference per-object CAS trees
- Comparing ObjectRef/chunk semantics with YYDS multi-file layout
- Answering "was inline bytes ever the default?" (no — CAS was recommended)

## Do not load

- Implementing or reviewing **current** `.yydb` / YDPG persistence
- WAL, `-shm`, or `FilePager` behavior
- New feature design for embedded YYDB storage

## Layout (historical)

```text
<db>.objects/
└── objects/
    └── ab/
        └── abcd…ef.bytes
```

Default BLAKE3 hash-2 paths. Rows hold `ObjectRef`; large files use chunk manifests.

## Policy (historical)

| Approach                     | Policy                               |
|------------------------------|--------------------------------------|
| Inline in `.yydb`            | Allowed for tiny values; not default |
| CAS `objects/hash-2/*.bytes` | Recommended                          |

Soft threshold: `INLINE_BYTES_MAX` (4 KiB). Large files: chunked manifest + range reads.

Vectors prefer CAS; ANN graphs are `ObjectKind::AnnSegment` objects.

Hot/cold tiering is runtime cache on the same CAS (`pin_object`, `evict_object`).

Full backup for `.yydb` is the main file plus wal/shm companions only. Blob trees belong to `.yydx` layouts.

## Example prompt

```text
Load yydb-bytes-storage. This design doc proposes <app>.yydb.objects/ — mark every statement that conflicts with current single-file .yydb.
```
