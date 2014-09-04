//! Layered UDF registry: catalog, session, host, and built-ins.

use std::collections::HashMap;
use std::sync::Arc;

use crate::runtime::{Budget, UdfContext, UdfImplementation, UdfInvocation};
use crate::{Result, UdfDefinition, UdfError, UdfIdentity, UdfValue};

/// Catalog-visible metadata without host callbacks.
#[derive(Debug, Clone)]
pub struct CatalogUdfEntry {
    /// Catalog contract.
    pub definition: UdfDefinition,
}

/// Session-local metadata for `micro` and temporary UDFs.
#[derive(Debug, Clone)]
pub struct SessionUdfEntry {
    /// Session contract.
    pub definition: UdfDefinition,
    /// Whether this entry may shadow catalog names.
    pub shadow: bool,
}

/// Installed implementation binding for a resolved definition.
#[derive(Clone)]
pub struct ImplementationEntry {
    /// Logical identity.
    pub identity: UdfIdentity,
    /// Runnable implementation.
    pub implementation: Arc<dyn UdfImplementation>,
}

/// Options controlling session registration behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegisterOptions {
    /// Allow this session entry to replace a catalog entry with the same logical id.
    pub shadow: bool,
}

impl RegisterOptions {
    /// Creates default non-shadowing options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allows explicit shadowing of catalog entries.
    pub fn shadowing() -> Self {
        Self { shadow: true }
    }
}

fn logical_key(identity: &UdfIdentity) -> String {
    identity.logical_id()
}

/// Persistent catalog registry.
#[derive(Debug, Default)]
pub struct CatalogUdfRegistry {
    entries: HashMap<String, CatalogUdfEntry>,
}

impl CatalogUdfRegistry {
    /// Registers a catalog UDF definition.
    pub fn register(&mut self, definition: UdfDefinition) -> Result<()> {
        definition.validate()?;
        let key = logical_key(&definition.identity);
        if self.entries.contains_key(&key) {
            return Err(UdfError::NameConflict);
        }
        self.entries.insert(key, CatalogUdfEntry { definition });
        Ok(())
    }

    /// Returns a catalog entry by logical id.
    pub fn get(&self, identity: &UdfIdentity) -> Option<&CatalogUdfEntry> {
        self.entries.get(&logical_key(identity))
    }

    /// Returns registered logical function names.
    pub fn names(&self) -> Vec<String> {
        self.entries
            .values()
            .map(|entry| entry.definition.identity.name().to_owned())
            .collect()
    }

    /// Returns the version registered for `name`, if any.
    pub fn version_for_name(&self, name: &str) -> Option<u32> {
        self.entries.values().find_map(|entry| {
            (entry.definition.identity.name() == name).then_some(entry.definition.identity.version)
        })
    }
}

/// Query-session registry for `micro` and temporary UDFs.
#[derive(Debug, Default)]
pub struct SessionUdfRegistry {
    entries: HashMap<String, SessionUdfEntry>,
}

impl SessionUdfRegistry {
    /// Registers a session-local UDF.
    pub fn register(&mut self, definition: UdfDefinition, options: RegisterOptions) -> Result<()> {
        definition.validate()?;
        let key = logical_key(&definition.identity);
        self.entries.insert(
            key,
            SessionUdfEntry {
                definition,
                shadow: options.shadow,
            },
        );
        Ok(())
    }

    /// Returns a session entry by logical id.
    pub fn get(&self, identity: &UdfIdentity) -> Option<&SessionUdfEntry> {
        self.entries.get(&logical_key(identity))
    }

    /// Returns registered logical function names.
    pub fn names(&self) -> Vec<String> {
        self.entries
            .values()
            .map(|entry| entry.definition.identity.name().to_owned())
            .collect()
    }

    /// Returns the version registered for `name`, if any.
    pub fn version_for_name(&self, name: &str) -> Option<u32> {
        self.entries.values().find_map(|entry| {
            (entry.definition.identity.name() == name).then_some(entry.definition.identity.version)
        })
    }

    /// Removes a session entry by logical function name.
    pub fn remove_by_name(&mut self, name: &str) -> Result<UdfIdentity> {
        let key = self
            .entries
            .iter()
            .find(|(_, entry)| entry.definition.identity.name() == name)
            .map(|(key, _)| key.clone())
            .ok_or(UdfError::NotFound)?;
        self.entries
            .remove(&key)
            .map(|entry| entry.definition.identity)
            .ok_or(UdfError::NotFound)
    }
}

/// Process-local registry for native, execution, and host-runtime implementations.
#[derive(Default)]
pub struct ImplementationRegistry {
    entries: HashMap<String, ImplementationEntry>,
}

impl ImplementationRegistry {
    /// Installs an implementation binding.
    pub fn install(&mut self, implementation: Arc<dyn UdfImplementation>) -> Result<()> {
        let key = logical_key(implementation.identity());
        if self.entries.contains_key(&key) {
            return Err(UdfError::NameConflict);
        }
        self.entries.insert(
            key,
            ImplementationEntry {
                identity: implementation.identity().clone(),
                implementation,
            },
        );
        Ok(())
    }

    /// Returns an implementation binding by logical id.
    pub fn get(&self, identity: &UdfIdentity) -> Option<&ImplementationEntry> {
        self.entries.get(&logical_key(identity))
    }

    /// Removes an implementation binding by logical id.
    pub fn remove(&mut self, identity: &UdfIdentity) {
        self.entries.remove(&logical_key(identity));
    }
}

/// Unified registry with `session > catalog > built-in` resolution.
#[derive(Default)]
pub struct UdfRegistry {
    catalog: CatalogUdfRegistry,
    session: SessionUdfRegistry,
    implementations: ImplementationRegistry,
    builtins: CatalogUdfRegistry,
}

impl UdfRegistry {
    /// Returns the mutable catalog registry.
    pub fn catalog_mut(&mut self) -> &mut CatalogUdfRegistry {
        &mut self.catalog
    }

    /// Returns the mutable session registry.
    pub fn session_mut(&mut self) -> &mut SessionUdfRegistry {
        &mut self.session
    }

    /// Returns the mutable implementation registry.
    pub fn implementation_mut(&mut self) -> &mut ImplementationRegistry {
        &mut self.implementations
    }

    /// Registers a built-in catalog entry.
    pub fn register_builtin(&mut self, definition: UdfDefinition) -> Result<()> {
        self.builtins.register(definition)
    }

    /// Registers a session UDF and rejects silent catalog shadowing.
    pub fn register_session(
        &mut self,
        definition: UdfDefinition,
        options: RegisterOptions,
    ) -> Result<()> {
        if !options.shadow
            && (self.catalog.get(&definition.identity).is_some()
                || self.builtins.get(&definition.identity).is_some())
        {
            return Err(UdfError::NameConflict);
        }
        self.session.register(definition, options)
    }

    /// Registers a session-local micro and binds its host implementation.
    pub fn register_session_micro(
        &mut self,
        definition: UdfDefinition,
        implementation: Arc<dyn UdfImplementation>,
        options: RegisterOptions,
    ) -> Result<()> {
        self.register_session(definition, options)?;
        self.bind_implementation(implementation)
    }

    /// Registers a catalog macro and binds its implementation.
    pub fn register_catalog_macro(
        &mut self,
        definition: UdfDefinition,
        implementation: Arc<dyn UdfImplementation>,
    ) -> Result<()> {
        self.catalog_mut().register(definition)?;
        self.bind_implementation(implementation)
    }

    /// Installs an implementation after verifying catalog or session metadata exists.
    pub fn bind_implementation(
        &mut self,
        implementation: Arc<dyn UdfImplementation>,
    ) -> Result<()> {
        let identity = implementation.identity();
        if self.resolve_definition(identity).is_none() {
            return Err(UdfError::InvalidDefinition);
        }
        self.implementations.install(implementation)
    }

    /// Resolves catalog metadata using `session > catalog > built-in` precedence.
    pub fn resolve_definition(&self, identity: &UdfIdentity) -> Option<&UdfDefinition> {
        if let Some(entry) = self.session.get(identity) {
            return Some(&entry.definition);
        }
        if let Some(entry) = self.catalog.get(identity) {
            return Some(&entry.definition);
        }
        self.builtins.get(identity).map(|entry| &entry.definition)
    }

    /// Returns logical function names registered in the catalog.
    pub fn catalog_names(&self) -> Vec<String> {
        let mut names = self.catalog.names();
        names.extend(self.builtins.names());
        names.sort();
        names.dedup();
        names
    }

    /// Returns logical function names registered in the current session.
    pub fn session_names(&self) -> Vec<String> {
        self.session.names()
    }

    /// Resolves the registered version for `name` using session, catalog, then built-ins.
    pub fn version_for_name(&self, name: &str) -> Option<u32> {
        self.session
            .version_for_name(name)
            .or_else(|| self.catalog.version_for_name(name))
            .or_else(|| self.builtins.version_for_name(name))
    }

    /// Removes a session-local UDF and any bound implementation.
    pub fn remove_session_by_name(&mut self, name: &str) -> Result<()> {
        let identity = self.session.remove_by_name(name)?;
        self.implementations.remove(&identity);
        Ok(())
    }

    /// Invokes a UDF through the implementation registry.
    pub fn invoke(&self, invocation: &UdfInvocation, budget: Budget) -> Result<UdfValue> {
        let binding = self
            .implementations
            .get(&invocation.identity)
            .ok_or(UdfError::ImplementationUnavailable)?;
        if binding.identity.version != invocation.identity.version {
            return Err(UdfError::VersionMismatch);
        }
        let signature = binding.implementation.signature();
        if let Some(expected) = signature.expected_arity() {
            if invocation.args.len() != expected {
                return Err(UdfError::ArityMismatch);
            }
        }
        signature.ensure_args(&UdfValue::argument_types(&invocation.args))?;
        let mut context = UdfContext::new(budget);
        binding
            .implementation
            .invoke(&mut context, &invocation.args)
    }
}
