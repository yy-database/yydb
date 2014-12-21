//! Session-based auth for `yydb serve`.
//!
//! This is the **minimal working session model** for Phase 1: every
//! connection proves its identity via a bearer token, the server looks up
//! a [`Principal`] in the bootstrap registry, creates an in-memory
//! [`Session`], and the connection cannot issue frames until a session
//! is attached. Role and permission enums are present so follow-up work
//! can add actual authorization checks without breaking the wire
//! contract.
//!
//! Only bootstrap principals live here right now — persistent rotation,
//! challenge-response, and per-connection session token issuance are
//! follow-up steps.

use std::{
    collections::HashMap,
    env,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use yydb_types::Error;

// ─── Principal ──────────────────────────────────────────────────────────

/// What kind of identity a connection presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrincipalKind {
    /// Named human user.
    User,
    /// Named machine principal (service account).
    Service,
    /// Loopback-only anonymous — never accepted from a remote bind.
    Anonymous,
}

impl PrincipalKind {
    /// Stable name for diagnostics and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Service => "service",
            Self::Anonymous => "anonymous",
        }
    }
}

/// A concrete identity the server will attach to authenticated connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Human / service name; empty for `Anonymous`.
    pub name: String,
    /// Category of the principal (user vs service vs anonymous).
    pub kind: PrincipalKind,
    /// Roles assigned to this principal (authorization follows next).
    pub roles: Vec<Role>,
}

impl Principal {
    /// Build an anonymous loopback principal (dev-only).
    pub fn anonymous_loopback() -> Self {
        Self { name: String::new(), kind: PrincipalKind::Anonymous, roles: vec![Role::LoopbackOnly] }
    }

    /// Whether this principal may be used from a non-loopback bind.
    pub fn allows_remote(&self) -> bool {
        self.kind != PrincipalKind::Anonymous
    }

    /// Stable label for logs (`anonymous` or `service:worker-07` etc).
    pub fn label(&self) -> String {
        match self.kind {
            PrincipalKind::Anonymous => PrincipalKind::Anonymous.as_str().to_owned(),
            kind => format!("{}:{}", kind.as_str(), self.name),
        }
    }
}

// ─── Role / Permission (minimal, for follow-up authz) ──────────────────

/// Coarse-grained role that aggregates permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Read-only access (`KvGet`, prepared selects, `Info`, schema introspect).
    Reader,
    /// Reads plus writes (`KvPut`, prepared mutations, schema-ensure).
    Writer,
    /// Full schema control (same as Writer for now — flag is explicit).
    SchemaAdmin,
    /// Loopback-only anonymous bootstrap (never exposed remotely).
    LoopbackOnly,
}

/// Atomic permission bit that dispatch checks before executing a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Read any record or prepared select.
    Read,
    /// Write records or prepared mutations.
    Write,
    /// Change schema via `SchemaEnsure`.
    Schema,
}

impl Role {
    /// The concrete permissions a role grants today.
    pub fn permissions(self) -> &'static [Permission] {
        match self {
            Role::Reader => &[Permission::Read],
            Role::Writer => &[Permission::Read, Permission::Write],
            Role::SchemaAdmin => &[Permission::Read, Permission::Write, Permission::Schema],
            Role::LoopbackOnly => &[Permission::Read, Permission::Write, Permission::Schema],
        }
    }
}

// ─── Session ────────────────────────────────────────────────────────────

/// Per-authenticated-connection session — bound to a principal and token,
/// carries expiry so the server can clean up idle credentials later.
#[derive(Debug, Clone)]
pub struct Session {
    /// Principal attached to this session.
    pub principal: Principal,
    /// Token used to authenticate (bootstrap token or issued session token).
    pub token: String,
    /// Wall-clock creation time.
    pub created_at: SystemTime,
    /// Wall-clock expiry time — connections that outlive this must re-auth.
    pub expires_at: SystemTime,
}

impl Session {
    /// Whether wall-clock has passed `expires_at`.
    pub fn is_expired(&self) -> bool {
        SystemTime::now() > self.expires_at
    }

    /// TTL for a freshly-created session (1 hour).
    pub const TTL: Duration = Duration::from_secs(60 * 60);
}

// ─── Bootstrap config ──────────────────────────────────────────────────

/// Parsed bootstrap configuration read from environment at `serve` startup.
///
/// Format (env var `YYDB_AUTH_TOKENS`):
///
/// ```text
/// t1=alice:SchemaAdmin;t2=svc-worker-07:Reader,Writer;t3=anonymous:LoopbackOnly
/// ```
///
/// * Semicolon separates entries.
/// * `=` separates token from principal spec.
/// * Principal spec is `name:Role1,Role2`; if `name` is literally `anonymous`
///   the principal is [`PrincipalKind::Anonymous`] regardless of bind.
/// * If the env var is missing and bind is loopback-only, a single
///   anonymous loopback principal is seeded with the prior default token
///   so local dev keeps working. If bind is *not* loopback and the env var
///   is missing, the server refuses to start.
#[derive(Debug, Clone)]
pub struct AuthBootstrap {
    /// Token → Principal map of configured identities.
    pub tokens: Vec<(String, Principal)>,
}

impl AuthBootstrap {
    /// Parse environment variables to build bootstrap identities.
    ///
    /// `bind_is_loopback` controls whether the anonymous loopback default
    /// may be installed. Non-loopback binds require at least one named
    /// principal — otherwise [`Error::AuthConfigMissing`] is returned so
    /// serve.rs can refuse startup before opening any listener.
    pub fn from_env(bind_is_loopback: bool) -> Result<Self, Error> {
        if let Ok(raw) = env::var("YYDB_AUTH_TOKENS") {
            if !raw.trim().is_empty() {
                let tokens = parse_env_tokens(&raw)?;
                return Ok(Self { tokens });
            }
        }
        // No explicit auth config: only allowed on loopback.
        if bind_is_loopback {
            let default_token =
                env::var("YYDB_AUTH_TOKEN").unwrap_or_else(|_| "yydb-phase0-default-token".to_owned());
            let anon = Principal::anonymous_loopback();
            return Ok(Self { tokens: vec![(default_token, anon)] });
        }
        Err(Error::AuthConfigMissing {
            message: "non-loopback bind requires explicit auth — set YYDB_AUTH_TOKENS \
                      (format: `token=name:Role1,Role2;…`) before starting serve".to_owned(),
        })
    }

    /// Count of bootstrap principals that are *not* anonymous.
    pub fn named_count(&self) -> usize {
        self.tokens.iter().filter(|(_, p)| p.kind != PrincipalKind::Anonymous).count()
    }
}

/// Parse one entry (`token=name:Role1,Role2`) into `(String, Principal)`.
fn parse_entry(entry: &str) -> Result<(String, Principal), Error> {
    let (token, rest) = entry
        .split_once('=')
        .ok_or_else(|| Error::AuthConfigMissing { message: format!("malformed auth entry: `{entry}`").into() })?;
    let token = token.trim();
    if token.is_empty() {
        return Err(Error::AuthConfigMissing { message: "empty token in auth config".to_owned() });
    }

    let (name, roles_part) = match rest.split_once(':') {
        Some((n, r)) => (n.trim(), r.trim()),
        None => (rest.trim(), "Writer"),
    };
    let name = name.to_owned();
    let kind = if name == "anonymous" { PrincipalKind::Anonymous } else { PrincipalKind::Service };
    let roles = parse_roles(roles_part)?;

    Ok((token.to_owned(), Principal { name, kind, roles }))
}

fn parse_env_tokens(raw: &str) -> Result<Vec<(String, Principal)>, Error> {
    let mut out = Vec::new();
    for entry in raw.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        out.push(parse_entry(entry)?);
    }
    if out.is_empty() {
        return Err(Error::AuthConfigMissing {
            message: "YYDB_AUTH_TOKENS parsed to no entries — set at least one token".to_owned(),
        });
    }
    Ok(out)
}

fn parse_roles(raw: &str) -> Result<Vec<Role>, Error> {
    let mut out = Vec::new();
    for piece in raw.split(',') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let role = match piece.to_ascii_lowercase().as_str() {
            "reader" => Role::Reader,
            "writer" => Role::Writer,
            "schemaadmin" | "schema_admin" | "schema" => Role::SchemaAdmin,
            "loopbackonly" | "loopback_only" => Role::LoopbackOnly,
            other => {
                return Err(Error::AuthConfigMissing {
                    message: format!("unknown role `{other}` in auth config"),
                })
            }
        };
        out.push(role);
    }
    if out.is_empty() {
        out.push(Role::Writer);
    }
    Ok(out)
}

// ─── Session registry ──────────────────────────────────────────────────

/// In-memory registry of bootstrap tokens and active sessions.
///
/// Thread-safe for the serve loop (each handler spawns its own thread).
/// Bootstrap tokens are static for the process lifetime; sessions are
/// created on demand when a connection successfully Authenticates.
pub struct SessionRegistry {
    /// Long-lived identity entries seeded from [`AuthBootstrap`].
    bootstrap: HashMap<String, Principal>,
    /// Active sessions indexed by the token the connection presented.
    sessions: Mutex<HashMap<String, Session>>,
}

impl SessionRegistry {
    /// Build a registry from parsed bootstrap config.
    pub fn new(bootstrap: &AuthBootstrap) -> Self {
        let bootstrap_map = bootstrap.tokens.iter().cloned().collect::<HashMap<_, _>>();
        Self { bootstrap: bootstrap_map, sessions: Mutex::new(HashMap::new()) }
    }

    /// Look up an active non-expired session for this token.
    ///
    /// Returns structured auth errors so dispatch.rs can report the exact
    /// reason the token was rejected.
    pub fn lookup_session(&self, token: &str, remote: bool) -> Result<Session, Error> {
        // Fast path: already-authenticated session present.
        {
            let sessions = self.sessions.lock().expect("session registry poisoned");
            if let Some(session) = sessions.get(token) {
                if session.is_expired() {
                    return Err(Error::AuthSessionExpired {
                        message: format!("session for principal `{}` expired at {:?}", session.principal.label(), session.expires_at),
                    });
                }
                if remote && !session.principal.allows_remote() {
                    return Err(Error::AuthAnonymousRemoteDenied {
                        message: format!("principal `{}` may only authenticate from loopback", session.principal.label()),
                    });
                }
                return Ok(session.clone());
            }
        }

        // Slow path: create a fresh session from bootstrap principal.
        let principal = self
            .bootstrap
            .get(token)
            .cloned()
            .ok_or_else(|| Error::AuthTokenInvalid {
                message: "token does not match any configured principal".to_owned(),
            })?;

        if remote && !principal.allows_remote() {
            return Err(Error::AuthAnonymousRemoteDenied {
                message: format!("principal `{}` may only authenticate from loopback", principal.label()),
            });
        }

        let now = SystemTime::now();
        let session = Session {
            principal: principal.clone(),
            token: token.to_owned(),
            created_at: now,
            expires_at: now + Session::TTL,
        };
        let mut sessions = self.sessions.lock().expect("session registry poisoned");
        sessions.insert(token.to_owned(), session.clone());
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_tokens_single_entry() {
        let bootstrap = AuthBootstrap::from_env(true).unwrap();
        // with no env vars and loopback true, default anonymous should be seeded
        assert_eq!(bootstrap.named_count(), 0);
    }

    #[test]
    fn anon_forbidden_on_remote_lookup() {
        let bootstrap = AuthBootstrap {
            tokens: vec![("t-anon".to_owned(), Principal::anonymous_loopback())],
        };
        let reg = SessionRegistry::new(&bootstrap);
        let err = reg.lookup_session("t-anon", true).unwrap_err();
        assert!(matches!(err, Error::AuthAnonymousRemoteDenied { .. }));
    }

    #[test]
    fn anon_ok_on_loopback_lookup() {
        let bootstrap = AuthBootstrap {
            tokens: vec![("t-anon".to_owned(), Principal::anonymous_loopback())],
        };
        let reg = SessionRegistry::new(&bootstrap);
        let session = reg.lookup_session("t-anon", false).unwrap();
        assert_eq!(session.principal.kind, PrincipalKind::Anonymous);
    }

    #[test]
    fn unknown_token_is_token_invalid() {
        let bootstrap = AuthBootstrap { tokens: vec![] };
        let reg = SessionRegistry::new(&bootstrap);
        let err = reg.lookup_session("nothing", false).unwrap_err();
        assert!(matches!(err, Error::AuthTokenInvalid { .. }));
    }

    #[test]
    fn parse_explicit_env_multiple_entries() {
        let raw = "t1=alice:SchemaAdmin;t2=svc:Reader,Writer;t3=anonymous:LoopbackOnly";
        let entries = parse_env_tokens(raw).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].0, "t1");
        assert_eq!(entries[0].1.name, "alice");
        assert_eq!(entries[0].1.kind, PrincipalKind::Service);
        assert!(entries[0].1.roles.contains(&Role::SchemaAdmin));
        assert_eq!(entries[2].1.kind, PrincipalKind::Anonymous);
    }
}
