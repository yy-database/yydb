# YYDB bytes storage: unified `objects/` CAS

## Non-goals

- No `.yydb-vec` (or any vector-only sidecar extension).
- No “save the whole large file as one giant blob” as the recommended path.

## Layout

Primary database truth stays in `.yydb` (+ optional `-wal` / `-shm`).

Binary payloads use one content-addressed store:

```text
<db>.objects/
└── objects/
    └── ab/                    # hash-2 (first two hex chars of the hash)
        └── abcd…ef.bytes      # full hex hash + ".bytes"
```

Default `<db>.objects` sits beside `app.yydb` (configurable root later). Hash algorithm default: **BLAKE3**;
`ObjectRef.algo` records which digester produced the name.

The same mechanism stores:

- vector payloads
- file chunks
- rebuildable ANN segments
- other opaque bytes

## Inline vs CAS

| Approach                           | Policy                                                                          |
|------------------------------------|---------------------------------------------------------------------------------|
| Inline into `.yydb` rows           | **Allowed**, useful for tiny values / tests; **not recommended** as the default |
| CAS under `objects/hash-2/*.bytes` | **Recommended**; rows keep `ObjectRef` (or a chunk manifest)                    |

Suggested soft threshold: `INLINE_BYTES_MAX` (4 KiB). Oversized inline is still permitted when the caller opts in;
prefer CAS for anything larger.

## Large files: always chunked

A logical file is a **manifest** (ordered list of chunk `ObjectRef`s) plus N chunk objects. Each chunk is a normal
`objects/…/*.bytes` entry.

- Writers: `put_file_chunked` → manifest + chunks.
- Readers: `read_file_range(offset, len)` maps to the touched chunks only (shard/chunk read). Do not require assembling
  the whole file first.
- A small file may be a single-chunk manifest (same API).

## Vectors

`vector<N>` is a first-class VOS / `Value` type. On disk, prefer CAS (vector payload encoded as `.bytes`) with row
metadata (dim, metric). ANN graphs are ordinary CAS objects (`ObjectKind::AnnSegment`), never a special file suffix.

## Runtime hot / cold tiering

Tiering is a **runtime cache policy on the same CAS**, not a second on-disk protocol:

- **Hot**: in-process cache / pinned objects (vector hot sets, recent chunks)
- **Cold**: on-disk `objects/…/*.bytes`
- APIs: `pin_object`, `evict_object`, `get_object` (fault-in)

Pinned/hot state is disposable; content hash remains authoritative.

## Copy / backup

| Needed for            | Artifacts                    |
|-----------------------|------------------------------|
| Structured truth only | `.yydb` (+ wal/shm if used)  |
| Full data plane       | above + `<db>.objects/` tree |

## YYDS alignment

YYDS nodes keep multi-file `.yyds` → `.yykv*`, but binary CAS/chunk/ObjectRef semantics should align with this document
so embeddings and files share one bytes model across products.
