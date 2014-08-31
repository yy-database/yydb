#!/usr/bin/env node
/**
 * Declarative reshape for the YYDB facade split.
 *
 * Default dry-run. `--apply` writes. `--stage=protocol|store|cleanup|all`.
 *
 * The script only moves original text and applies asserted textual replacements.
 * It never regenerates Rust implementations. Mixed files (`wire.rs`) are split at
 * exact declaration markers, not by broad regular expressions.
 */
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const apply = process.argv.includes('--apply');
const stageArg = process.argv.find((arg) => arg.startsWith('--stage='));
const stage = stageArg ? stageArg.split('=')[1] : 'all';

const crates = join(root, 'projects', 'crates');
const facadeSrc = join(crates, 'yydb', 'src');
const storeDir = join(crates, 'yydb-store');
const protocolDir = join(crates, 'yydb-protocol');
const serverDir = join(crates, 'yydb-server');

const plan = { moves: [], writes: [], edits: [], deletes: [], notes: [] };

const rel = (path) => relative(root, path).split('\\').join('/');
const read = (path) => readFileSync(path, 'utf8');
const hasStage = (name) => stage === 'all' || stage === name;

function fail(message) {
  throw new Error(message);
}

function insideRepo(path) {
  const r = relative(root, path);
  if (r.startsWith('..') || r === '') fail(`path escapes the repository: ${path}`);
}

function planMove(from, to) {
  insideRepo(from);
  insideRepo(to);
  if (!existsSync(from)) {
    if (existsSync(to)) {
      plan.notes.push(`already moved: ${rel(from)} -> ${rel(to)}`);
      return;
    }
    fail(`missing source: ${rel(from)}`);
  }
  if (existsSync(to)) fail(`target already exists: ${rel(to)}`);
  plan.moves.push({ from, to });
}

function planWrite(path, content) {
  insideRepo(path);
  if (existsSync(path)) {
    if (read(path) === content) {
      plan.notes.push(`already current: ${rel(path)}`);
      return;
    }
    fail(`generated file already exists with different content: ${rel(path)}`);
  }
  plan.writes.push({ path, content });
}

function planOverwrite(path, content, expectedMarker) {
  insideRepo(path);
  if (!existsSync(path)) {
    plan.writes.push({ path, content });
    return;
  }
  const current = read(path);
  if (current === content) {
    plan.notes.push(`already current: ${rel(path)}`);
    return;
  }
  if (!current.includes(expectedMarker)) {
    fail(`refusing to overwrite ${rel(path)}: expected marker ${JSON.stringify(expectedMarker)} not found`);
  }
  plan.writes.push({ path, content });
}

function planDelete(path) {
  insideRepo(path);
  if (!existsSync(path)) {
    plan.notes.push(`already deleted: ${rel(path)}`);
    return;
  }
  plan.deletes.push({ path });
}

function applyReplacements(path, replacements, label = rel(path)) {
  const source = read(path);
  const crlf = source.includes('\r\n');
  const toEol = (text) => (crlf ? text.replace(/\r\n/g, '\n').replace(/\n/g, '\r\n') : text);
  let next = source;
  for (const item of replacements) {
    const search = toEol(item.search);
    const replace = toEol(item.replace);
    const found = next.split(search).length - 1;
    if (found === 0 && next.includes(replace)) continue;
    if (found !== item.expected) {
      fail(`expected ${item.expected} hit(s) for ${JSON.stringify(search.slice(0, 60))} in ${label}, found ${found}`);
    }
    next = next.split(search).join(replace);
  }
  return next === source ? null : next;
}

function planEdit(path, replacements) {
  insideRepo(path);
  if (!existsSync(path)) fail(`missing edit target: ${rel(path)}`);
  const next = applyReplacements(path, replacements);
  if (next !== null) plan.edits.push({ path, content: next });
}

function planEditMoved(source, target, replacements) {
  insideRepo(source);
  insideRepo(target);
  if (!existsSync(source)) {
    plan.notes.push(`already moved past edit target: ${rel(source)}`);
    return;
  }
  const next = applyReplacements(source, replacements, `${rel(source)} -> ${rel(target)}`);
  if (next !== null) plan.edits.push({ path: target, content: next });
}

function listFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...listFiles(full));
    else out.push(full);
  }
  return out;
}

// ---------------------------------------------------------------------------
// Shared generated crate manifests and library roots
// ---------------------------------------------------------------------------

const protocolManifest = `[package]
name = "yydb-protocol"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
description = "YY wire frame and message-body codec shared by yydb serve and clients"
homepage = "https://github.com/yy-database/yydb.rs"
documentation = "https://docs.rs/yydb-protocol"
keywords = ["database", "protocol", "wire"]
categories = ["database-implementations"]

[dependencies]
yydb-types = { workspace = true }
yydb-udf = { workspace = true }
`;

const storeManifest = `[package]
name = "yydb-store"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
description = "YYDB storage engine: connection, catalog, CAS, journal and UDF session state"
homepage = "https://github.com/yy-database/yydb.rs"
documentation = "https://docs.rs/yydb-store"
keywords = ["database", "embedded", "storage", "vos"]
categories = ["database-implementations", "embedded"]

[dependencies]
vos = { workspace = true }
yydb-types = { workspace = true }
yydb-format = { workspace = true }
yydb-execution = { workspace = true }
yydb-udf = { workspace = true }
serde_json = { workspace = true }

[target.'cfg(windows)'.dependencies]
windows-sys = { workspace = true }

[target.'cfg(unix)'.dependencies]
libc = { version = "0.2", default-features = false }
`;

const typeExports = `    ChunkManifest, CommitSequence, CompactReport, DoctorIssue, DoctorReport, DoctorSeverity, Error,
    EvictBudget, EvictReport, EvictionPolicy, HashAlgo, LeaseExpectation, LeaseToken,
    NamespaceQuota, NamespaceStats, ObjectKind, ObjectRef, ReclaimReport, RecordVersion,
    ReleaseOutcome, Result, SchemaVersion, Tier, Value, Vector, DEFAULT_CHUNK_SIZE,
    INLINE_BYTES_MAX,`;

const storeLib = `//! YYDB storage engine: durable connection state, VOS catalog publication, CAS,
//! journal/WAL, lease/TTL and the UDF session bridge.
//!
//! This crate owns byte-level implementation. The \`yydb\` facade re-exports it and
//! must not grow implementations of its own.

#![deny(missing_docs)]

/// Batched write operations.
pub mod batch;
/// VOS catalog publication and contract resolution.
pub mod catalog;
/// Durable connection handle and statement surface.
pub mod connection;
/// Journal modes and \`-wal\` / \`-shm\` sidecar helpers.
pub mod journal;
/// Unified \`<hash-prefix>/<chunk_hash>.blob\` CAS + hot/cold tiering.
pub mod objects;
/// UDF traits (\`ScalarUdf\`) and registry types (\`UdfRegistry\`, ...).
pub mod udf;

mod doctor;
mod file_lock;
mod format_v0;
mod lease;
mod refs;
mod ttl;
mod udf_bridge;

pub use batch::Batch;
/// Historic module alias kept for \`yydb::schema\` compatibility.
pub use catalog as schema;
pub use connection::Connection;
pub use journal::{JournalMode, OpenFlags};
pub use objects::ObjectStore;
pub use udf::ScalarUdf;

pub use udf::{
    HostFunctionHandle, HostMicroDefinition, HostRuntimeAdapter, UdfError, UdfRegistry, UdfType,
    UdfValue,
};
/// Result alias for UDF host functions and registry operations.
pub type UdfResult<T = UdfValue> = yydb_udf::Result<T>;

pub use yydb_execution as execution;
pub use yydb_types;
pub use yydb_udf;
pub use yydb_types::{
${typeExports}
};

/// Storage-engine version string (Cargo package version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
`;

const facadeLib = `//! YYDB public facade -- Rust embed \`Connection\` with **VOS as the only DDL and
//! query language**.
//!
//! ## Crate layering
//!
//! | Layer | Crate | Role |
//! |-------|-------|------|
//! | Bottom | \`yydb-types\` | \`Error\`, \`Value\`, CAS refs |
//! | Engine | \`yydb-store\`, \`yydb-format\`, \`yydb-execution\`, \`yydb-udf\`, \`yydb-query\` | Storage, formats, execution IR, UDF, query |
//! | **Facade** | **\`yydb\`** | Re-exports, prelude, version |
//! | Protocol | \`yydb-protocol\` | Wire frames and bodies (no database access) |
//! | Transport | \`yydb-client\`, \`yydb-server\` | Remote client / serve loop |
//! | Bindings | \`yydb-napi\`, \`yydb-pyo3\`, ... | Language hosts only |
//!
//! **Embedded host:** register native [UDFs](udf) on [\`Connection\`]. Remote Rust
//! hosts use [\`yydb-client\`]. Lightweight TS hosts use \`@yydb/yydb-client\` against
//! \`yydb-server\`.
//!
//! Durable layout is still **one primary \`.yydb\` file**. Optional journal
//! sidecars \`{path}-wal\` / \`{path}-shm\` appear when
//! [\`JournalMode::Wal\`](journal::JournalMode::Wal) is enabled. Binary payloads
//! use the unified [\`objects\`] CAS (\`<hash-prefix>/<chunk_hash>.blob\`) with optional
//! runtime hot/cold tiering -- not a \`.yydb-vec\` sidecar.
//!
//! \`\`\`rust,no_run
//! use yydb::{Connection, OpenFlags, Result, Value};
//!
//! fn main() -> Result<()> {
//!     let conn = Connection::open_with_flags("app.yydb", OpenFlags::wal())?;
//!     conn.ensure_schema("table Project { @@id: uuid, title: utf8 }")?;
//!     conn.create_scalar("double", 1, |args| match args {
//!         [Value::I64(n)] => Ok(Value::I64(n * 2)),
//!         _ => Err(yydb::Error::Udf {
//!             name: "double".into(),
//!             message: "expected i64".into(),
//!         }),
//!     })?;
//!     conn.checkpoint()?;
//!     Ok(())
//! }
//! \`\`\`

#![deny(missing_docs)]

/// Shared types (\`Error\`, \`Result\`, \`Value\`, ...).
pub mod types {
    pub use yydb_types::*;
}

/// Journal modes and \`-wal\` / \`-shm\` sidecar helpers.
pub use yydb_store::journal;

/// Unified \`<hash-prefix>/<chunk_hash>.blob\` CAS + hot/cold tiering.
pub use yydb_store::objects;

/// Shared VOS schema contract.
pub use yydb_store::catalog as schema;

/// UDF traits (\`ScalarUdf\`) and registry types (\`UdfRegistry\`, ...).
pub use yydb_store::udf;

/// Common embed imports (\`use yydb::prelude::*\`).
pub mod prelude {
    pub use crate::{
        Batch, Connection, Error, JournalMode, ObjectRef, ObjectStore, OpenFlags, Result,
        SchemaVersion, Value,
    };
}

/// Batched write operations.
pub use yydb_store::batch;

/// Durable connection handle and statement surface.
pub use yydb_store::connection;

/// YY wire protocol (\`YYDB\`|\`YYDS\` + version digits \`0000\`...).
pub use yydb_protocol as wire;

/// Query planning and DML operations.
pub use yydb_query as query;

pub use yydb_store::{
    Batch, Connection, HostFunctionHandle, HostMicroDefinition, HostRuntimeAdapter,
    JournalMode, ObjectStore, OpenFlags, ScalarUdf, UdfError, UdfRegistry, UdfType, UdfValue,
};
/// Result alias for UDF host functions and registry operations.
pub type UdfResult<T = UdfValue> = yydb_udf::Result<T>;

pub use yydb_execution as execution;
pub use yydb_types;
pub use yydb_udf;
pub use yydb_types::{
${typeExports}
};

/// VOS schema language facade.
pub use vos;

/// Library version string (Cargo package version).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
`;

// ---------------------------------------------------------------------------
// Stage: protocol
// ---------------------------------------------------------------------------

function planProtocol() {
  const wirePath = join(facadeSrc, 'wire.rs');
  if (!existsSync(wirePath)) {
    plan.notes.push('protocol stage already applied (wire.rs is gone)');
    return;
  }
  const wire = read(wirePath);
  const dispatchMarker = 'fn info_body(conn: &Connection) -> Result<Vec<u8>> {';
  const codecResumeMarker = '/// Encode a `SchemaEnsure` body.';
  const loopbackMarker = '/// Whether a `--bind` host is loopback';
  const splitBindMarker = '/// Split `host:port` / `[::1]:port` into host and port string.';
  const bodyStartMarker = 'use std::io::{Read, Write};';

  const dispatchAt = wire.indexOf(dispatchMarker);
  const resumeAt = wire.indexOf(codecResumeMarker);
  const loopbackAt = wire.indexOf(loopbackMarker);
  const splitAt = wire.indexOf(splitBindMarker);
  const bodyAt = wire.indexOf(bodyStartMarker);
  for (const [name, index] of [
    ['dispatch', dispatchAt],
    ['codec resume', resumeAt],
    ['loopback', loopbackAt],
    ['split_bind', splitAt],
    ['body start', bodyAt],
  ]) {
    if (index < 0) fail(`wire.rs marker not found: ${name}`);
  }

  const dispatchBlock = wire.slice(dispatchAt, resumeAt);
  const loopbackBlock = wire.slice(loopbackAt, splitAt);
  const splitBindBlock = wire.slice(splitAt);

  let codec = wire.slice(bodyAt, dispatchAt) + wire.slice(resumeAt, loopbackAt) + splitBindBlock;
  codec = codec.replace('use crate::Connection;\n', '');
  if (codec.includes('use crate::Connection;')) fail('failed to drop crate::Connection from protocol');

  const publicized = [
    ['fn error_frame(request_id: u32, error: &Error) -> Frame {', '/// Build an `Error` reply frame.\npub fn error_frame(request_id: u32, error: &Error) -> Frame {'],
    ['fn encode_schema_ok(schema: Option<SchemaVersion>) -> Vec<u8> {', '/// Encode a `SchemaGetOk` body.\npub fn encode_schema_ok(schema: Option<SchemaVersion>) -> Vec<u8> {'],
    ['fn decode_schema_ensure(body: &[u8]) -> Result<(u32, String)> {', '/// Decode a `SchemaEnsure` body.\npub fn decode_schema_ensure(body: &[u8]) -> Result<(u32, String)> {'],
    ['fn decode_kv_get(body: &[u8]) -> Result<String> {', '/// Decode a `KvGet` body.\npub fn decode_kv_get(body: &[u8]) -> Result<String> {'],
    ['fn decode_kv_put(body: &[u8]) -> Result<(String, Vec<u8>)> {', '/// Decode a `KvPut` body.\npub fn decode_kv_put(body: &[u8]) -> Result<(String, Vec<u8>)> {'],
    ['fn encode_kv_get_ok(value: Option<Vec<u8>>) -> Vec<u8> {', '/// Encode a `KvGetOk` body.\npub fn encode_kv_get_ok(value: Option<Vec<u8>>) -> Vec<u8> {'],
  ];
  for (const [search, replace] of publicized) {
    const hits = codec.split(search).length - 1;
    if (hits !== 1) fail(`expected 1 protocol declaration for ${search}, found ${hits}`);
    codec = codec.replace(search, replace);
  }

  const protocolLib = `//! YY wire protocol: frame layout, message bodies and the \`0000\` version prefix.
//!
//! Shared by \`yydb-server\` and \`yydb-client\`. This crate never opens a database,
//! never dispatches a request and never depends on the embedded engine.

#![deny(missing_docs)]

${codec}`;

  planWrite(join(protocolDir, 'Cargo.toml'), protocolManifest);
  planWrite(join(protocolDir, 'src', 'lib.rs'), protocolLib);

  let dispatchBody = dispatchBlock.replace('crate::version()', 'env!("CARGO_PKG_VERSION")');
  if (dispatchBody === dispatchBlock) fail('dispatch did not reference crate::version()');
  const dispatchLib = `//! Request dispatch for \`yydb serve\`: wire frames mapped onto a \`Connection\`.
//!
//! Kept out of \`yydb-protocol\` because it executes database operations.

use yydb_protocol::{
    decode_kv_get, decode_kv_put, decode_micro_register, decode_scalar_call, decode_schema_ensure,
    encode_kv_get_ok, encode_scalar_call_ok, encode_schema_ok, error_frame, Frame, MessageType,
};
use yydb::{Connection, Error, Result};

${dispatchBody}
${loopbackBlock}`;
  planWrite(join(serverDir, 'src', 'dispatch.rs'), dispatchLib);

  planEdit(join(facadeSrc, 'lib.rs'), [
    { search: '/// YY wire protocol (`YYDB`|`YYDS` + version digits `0000`…).\npub mod wire;\n', replace: '/// YY wire protocol (`YYDB`|`YYDS` + version digits `0000`…).\npub use yydb_protocol as wire;\n', expected: 1 },
  ]);

  planEdit(join(crates, 'yydb', 'Cargo.toml'), [
    { search: 'yydb-udf = { workspace = true }\nserde_json', replace: 'yydb-udf = { workspace = true }\nyydb-protocol = { workspace = true }\nserde_json', expected: 1 },
    { search: '[dev-dependencies]\nyydb-format = { workspace = true }\n', replace: '[dev-dependencies]\nyydb-format = { workspace = true }\nyydb-server = { workspace = true }\n', expected: 1 },
  ]);

  planEdit(join(serverDir, 'Cargo.toml'), [
    { search: 'yydb = { workspace = true }', replace: 'yydb = { workspace = true }\nyydb-protocol = { workspace = true }', expected: 1 },
  ]);

  planEdit(join(serverDir, 'src', 'lib.rs'), [
    { search: '//! Depends on the [`yydb`] facade only. Embedded Rust hosts use `yydb::Connection`\n//! directly. Remote hosts connect through `yydb-client` / `@yydb/yydb-client`.', replace: '//! Depends on `yydb-store` for execution and `yydb-protocol` for framing.\n//! Embedded Rust hosts use `yydb::Connection` directly. Remote hosts connect\n//! through `yydb-client` / `@yydb/yydb-client`.', expected: 1 },
    { search: 'pub mod host_wire;\n\nmod serve;\n\npub use serve::{assert_bind_allowed, run_serve};', replace: 'pub mod dispatch;\npub mod host_wire;\n\nmod serve;\n\npub use dispatch::{dispatch, is_loopback_host};\npub use serve::{assert_bind_allowed, run_serve};', expected: 1 },
  ]);

  planEdit(join(serverDir, 'src', 'serve.rs'), [
    { search: 'use yydb::wire::{self, dispatch, read_frame, write_frame, Frame};\nuse yydb::{Connection, Error};', replace: 'use yydb::{Connection, Error};\nuse yydb_protocol::{read_frame, split_bind, write_frame, Frame, HEADER_LEN, MAX_BODY_LEN};\n\nuse crate::dispatch::{dispatch, is_loopback_host};', expected: 1 },
    { search: 'wire::split_bind', replace: 'split_bind', expected: 2 },
    { search: 'wire::is_loopback_host', replace: 'is_loopback_host', expected: 2 },
    { search: 'wire::MAX_BODY_LEN', replace: 'MAX_BODY_LEN', expected: 1 },
    { search: 'wire::HEADER_LEN', replace: 'HEADER_LEN', expected: 1 },
  ]);

  planEdit(join(serverDir, 'src', 'host_wire.rs'), [
    { search: 'use yydb::wire::{\n    self, decode_micro_host_invoke_ok, encode_micro_host_invoke, read_frame, write_frame, Frame,\n    MessageType,\n};\nuse yydb::{HostFunctionHandle, HostRuntimeAdapter, UdfError, UdfResult, UdfValue};', replace: 'use yydb_protocol::{\n    decode_error, decode_micro_host_invoke_ok, encode_micro_host_invoke, read_frame, write_frame,\n    Frame, MessageType,\n};\nuse yydb::{HostFunctionHandle, HostRuntimeAdapter, UdfError, UdfResult, UdfValue};', expected: 1 },
    { search: 'wire::decode_error', replace: 'decode_error', expected: 1 },
  ]);

  planEdit(join(crates, 'yydb-client', 'Cargo.toml'), [
    { search: 'yydb = { workspace = true }', replace: 'yydb-types = { workspace = true }\nyydb-udf = { workspace = true }\nyydb-protocol = { workspace = true }\n\n[dev-dependencies]\nyydb = { workspace = true }\nyydb-server = { workspace = true }', expected: 1 },
  ]);

  planEdit(join(crates, 'yydb-client', 'src', 'lib.rs'), [
    { search: 'pub use yydb::{Error, HostMicroDefinition, Result, SchemaVersion, Value};', replace: 'pub use yydb_types::{Error, Result, SchemaVersion, Value};\npub use yydb_udf::HostMicroDefinition;', expected: 1 },
    { search: 'use yydb::wire::{\n    self, decode_error, decode_kv_get_ok, decode_scalar_call_ok, decode_schema_get_ok,\n    encode_kv_get, encode_kv_put, encode_micro_register, encode_scalar_call, encode_schema_ensure,\n    read_frame, write_frame, Frame, MessageType,\n};', replace: 'use yydb_protocol::{\n    decode_error, decode_kv_get_ok, decode_scalar_call_ok, decode_schema_get_ok, encode_kv_get,\n    encode_kv_put, encode_micro_register, encode_scalar_call, encode_schema_ensure, read_frame,\n    split_bind, write_frame, Frame, MessageType,\n};', expected: 1 },
    { search: 'wire::split_bind(trimmed)', replace: 'split_bind(trimmed)', expected: 1 },
  ]);

  planEdit(join(crates, 'yydb-client', 'tests', 'tcp_roundtrip.rs'), [
    { search: 'use yydb::{wire::dispatch, Connection, Value};\nuse yydb_client::{Client, Error};', replace: 'use yydb::{Connection, Value};\nuse yydb_client::{Client, Error};\nuse yydb_protocol::{read_frame, write_frame};\nuse yydb_server::dispatch;', expected: 1 },
    { search: 'yydb::wire::read_frame', replace: 'read_frame', expected: 2 },
    { search: 'yydb::wire::write_frame', replace: 'write_frame', expected: 2 },
  ]);

  planEdit(join(crates, 'yydb', 'tests', 'integration', 'wire_dispatch.rs'), [
    { search: '        decode_scalar_call_ok, dispatch, encode_kv_get, encode_micro_host_invoke,\n        encode_micro_host_invoke_ok, encode_micro_register, encode_scalar_call,\n        encode_schema_ensure, is_loopback_host, Frame, MessageType, MAGIC_YYDB, MAGIC_YYDS,\n        WIRE_VERSION,\n    },\n    Connection, HostFunctionHandle, UdfValue, Value,\n};', replace: '        decode_scalar_call_ok, encode_kv_get, encode_micro_host_invoke,\n        encode_micro_host_invoke_ok, encode_micro_register, encode_scalar_call,\n        encode_schema_ensure, Frame, MessageType, MAGIC_YYDB, MAGIC_YYDS, WIRE_VERSION,\n    },\n    Connection, HostFunctionHandle, UdfValue, Value,\n};\nuse yydb_server::{dispatch, is_loopback_host};', expected: 1 },
  ]);

  planDelete(wirePath);
}

// ---------------------------------------------------------------------------
// Stage: store
// ---------------------------------------------------------------------------

function planStore() {
  if (!existsSync(join(facadeSrc, 'connection'))) {
    plan.notes.push('store stage already applied (connection/ is gone)');
    return;
  }
  planMove(join(facadeSrc, 'schema.rs'), join(storeDir, 'src', 'catalog.rs'));
  for (const file of listFiles(facadeSrc)) {
    const name = relative(facadeSrc, file);
    if (name === 'lib.rs' || name === 'wire.rs' || name === 'schema.rs') continue;
    planMove(file, join(storeDir, 'src', name));
  }

  planWrite(join(storeDir, 'Cargo.toml'), storeManifest);
  planWrite(join(storeDir, 'src', 'lib.rs'), storeLib);
  planOverwrite(join(crates, 'yydb', 'src', 'lib.rs'), facadeLib, 'pub use yydb_protocol as wire;');

  planEditMoved(
    join(facadeSrc, 'connection', 'schema_api.rs'),
    join(storeDir, 'src', 'connection', 'schema_api.rs'),
    [{ search: 'use crate::schema;', replace: 'use crate::catalog as schema;', expected: 1 }],
  );

  planEdit(join(root, 'Cargo.toml'), [
    { search: 'yydb-udf = { version = "0.1.0", path = "projects/crates/yydb-udf" }\n', replace: 'yydb-udf = { version = "0.1.0", path = "projects/crates/yydb-udf" }\nyydb-query = { version = "0.1.0", path = "projects/crates/yydb-query" }\nyydb-store = { version = "0.1.0", path = "projects/crates/yydb-store" }\n', expected: 1 },
  ]);

  planEdit(join(crates, 'yydb', 'Cargo.toml'), [
    { search: 'yydb-protocol = { workspace = true }\nserde_json = { workspace = true }\nyydb-format = { workspace = true }\n\n[dev-dependencies]\nyydb-format = { workspace = true }\nyydb-server = { workspace = true }\n\n[target.\'cfg(windows)\'.dependencies]\nwindows-sys = { workspace = true }\n\n[target.\'cfg(unix)\'.dependencies]\nlibc = { version = "0.2", default-features = false }\n', replace: 'yydb-protocol = { workspace = true }\nyydb-store = { workspace = true }\nyydb-query = { workspace = true }\n\n[dev-dependencies]\nyydb-format = { workspace = true }\nyydb-server = { workspace = true }\n', expected: 1 },
  ]);

  planEdit(join(serverDir, 'Cargo.toml'), [
    { search: 'yydb = { workspace = true }\nyydb-protocol = { workspace = true }', replace: 'yydb-store = { workspace = true }\nyydb-protocol = { workspace = true }\nyydb-types = { workspace = true }\nyydb-udf = { workspace = true }', expected: 1 },
  ]);

  planEdit(join(serverDir, 'src', 'dispatch.rs'), [
    { search: 'use yydb::{Connection, Error, Result};', replace: 'use yydb_store::Connection;\nuse yydb_types::{Error, Result};', expected: 1 },
  ]);

  planEdit(join(serverDir, 'src', 'serve.rs'), [
    { search: 'use yydb::{Connection, Error};', replace: 'use yydb_store::Connection;\nuse yydb_types::Error;', expected: 1 },
  ]);

  planEdit(join(serverDir, 'src', 'host_wire.rs'), [
    { search: 'use yydb::{HostFunctionHandle, HostRuntimeAdapter, UdfError, UdfResult, UdfValue};', replace: 'use yydb_types::Error;\nuse yydb_udf::{HostFunctionHandle, HostRuntimeAdapter, Result as UdfResult, UdfError, UdfValue};', expected: 1 },
    { search: 'fn map_wire_error(error: yydb::Error) -> UdfError {', replace: 'fn map_wire_error(error: Error) -> UdfError {', expected: 1 },
  ]);
}

// ---------------------------------------------------------------------------
// Stage: cleanup (S5 drift paths)
// ---------------------------------------------------------------------------

function planCleanup() {
  const objectsPath = join(storeDir, 'src', 'objects.rs');
  if (!existsSync(objectsPath)) {
    plan.notes.push('cleanup stage skipped: yydb-store/src/objects.rs does not exist yet');
    return;
  }
  planEdit(objectsPath, [
    { search: '//! Production layout: `.yydx` uses `<app-name>-objects/` with `YYBB` segments.\n//! `open_beside_db` (`<db>.objects`) is a prototype drift path. `.yydb` single-\n//! file mode must not rely on persistent sidecars.', replace: '//! Production layout: `.yydx` uses `<app-name>-objects/` with `YYBB` segments.\n//! `.yydb` single-file mode must not rely on persistent sidecars.', expected: 1 },
    { search: '    /// Open or create the store next to `db_path` (`app.yydb` → `app.yydb.objects`).\n    ///\n    /// Prototype layout only. `Connection::open` does not call this path.\n    #[deprecated(note = "prototype drift layout; use open_ephemeral for .yydb or .yydx blob roots")]\n    pub fn open_beside_db(db_path: &Path) -> Result<Self> {\n        let mut os = db_path.as_os_str().to_owned();\n        os.push(".objects");\n        let root = PathBuf::from(os);\n        fs::create_dir_all(&root)?;\n        Ok(Self {\n            root: ObjectStoreRoot::OnDisk(root),\n            hot: Mutex::new(HashMap::new()),\n            cold: Mutex::new(HashMap::new()),\n            catalog: Mutex::new(HashMap::new()),\n            pinned: Mutex::new(HashSet::new()),\n        })\n    }\n\n', replace: '', expected: 1 },
  ]);
}

// ---------------------------------------------------------------------------
// Plan, report, apply
// ---------------------------------------------------------------------------

try {
  if (hasStage('protocol')) planProtocol();
  if (hasStage('store')) planStore();
  if (hasStage('cleanup')) planCleanup();
} catch (error) {
  console.error(`reshape failed: ${error.message}`);
  process.exit(1);
}

console.log(`reshape plan (${apply ? 'apply' : 'dry-run'}, stage=${stage})`);
for (const move of plan.moves) console.log(`  move   ${rel(move.from)} -> ${rel(move.to)}`);
for (const write of plan.writes) console.log(`  write  ${rel(write.path)}`);
for (const edit of plan.edits) console.log(`  edit   ${rel(edit.path)}`);
for (const del of plan.deletes) console.log(`  delete ${rel(del.path)}`);
for (const note of plan.notes) console.log(`  note   ${note}`);
if (plan.moves.length + plan.writes.length + plan.edits.length + plan.deletes.length === 0) {
  console.log('  (nothing to do)');
  process.exit(0);
}
if (!apply) {
  console.log('re-run with --apply to write these changes');
  process.exit(0);
}

for (const move of plan.moves) {
  mkdirSync(dirname(move.to), { recursive: true });
  renameSync(move.from, move.to);
}
for (const edit of plan.edits) writeFileSync(edit.path, edit.content);
for (const write of plan.writes) {
  mkdirSync(dirname(write.path), { recursive: true });
  writeFileSync(write.path, write.content);
}
for (const del of plan.deletes) unlinkSync(del.path);
console.log(`applied ${plan.moves.length} move(s), ${plan.writes.length} write(s), ${plan.edits.length} edit(s), ${plan.deletes.length} delete(s)`);
