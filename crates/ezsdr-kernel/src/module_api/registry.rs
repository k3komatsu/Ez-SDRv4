//! Explicit registration, compatibility checks and lookup (MA-31…MA-35).

use std::collections::BTreeMap;

use crate::spec::{Ident, Key, KeyDecl, Namespace};

use super::{
    CompileRule, Deployment, Factories, KERNEL_API, LinkDescriptor, ModuleDescriptor, ModuleError,
    ModuleErrorKind, ModuleRef, ModuleRegistry, Role, VerbDecl, VocabularyDescriptor,
};

impl Factories {
    /// Whether a factory for `role` is present (MA-31).
    pub fn has(self, role: Role) -> bool {
        match role {
            Role::Provider => self.provider,
            Role::Executor => self.executor,
            Role::Sink => self.sink,
            Role::Link => self.link,
            Role::Authority => self.authority,
        }
    }

    /// Every role this registration supplies (MA-31).
    pub fn roles(self) -> Vec<Role> {
        [
            Role::Provider,
            Role::Executor,
            Role::Sink,
            Role::Link,
            Role::Authority,
        ]
        .into_iter()
        .filter(|r| self.has(*r))
        .collect()
    }
}

impl ModuleRegistry {
    /// An empty registry (MA-32).
    pub fn new() -> ModuleRegistry {
        ModuleRegistry::default()
    }

    /// Registers a Vocabulary, whose keys, event kinds, verbs and checks the Kernel
    /// then enforces without interpreting them (MA-35).
    pub fn register_vocabulary(&mut self, v: VocabularyDescriptor) -> Result<(), ModuleError> {
        if self.vocabularies.contains_key(&v.id) {
            return Err(ModuleError::rejected(format!(
                "MA-32: vocabulary {} is already registered",
                v.id
            )));
        }
        // RS-14: a Session burst's target is resolved at `compile`, so `RejectAtPlan`
        // names a stage this Action never passes through and the verb would be
        // undispatchable (SC-27, RS-51).
        for verb in &v.verbs {
            if let CompileRule::TxBurst {
                late_policy: crate::stream::LatePolicy::RejectAtPlan,
                ..
            } = verb.compiles_to
            {
                return Err(ModuleError::rejected(format!(
                    "RS-14: verb {} compiles to a TxBurst with `RejectAtPlan`, which a Session \
                     burst never reaches",
                    verb.verb
                )));
            }
        }
        self.vocabularies.insert(v.id.clone(), v);
        Ok(())
    }

    /// Registers a Module. Fails when `kernel_api.major` differs from the Kernel's;
    /// a declared Vocabulary is absent or incompatible; a role has no factory or a
    /// factory no role; `(id, version)` is already registered; or `deployment` is
    /// `Plugin`, which is `Unsupported` in Phase 1.
    ///
    /// Rule: MA-31, MA-32.
    pub fn register(
        &mut self,
        d: ModuleDescriptor,
        factories: Factories,
    ) -> Result<(), ModuleError> {
        if let Deployment::Plugin { .. } = d.deployment {
            return Err(ModuleError {
                kind: ModuleErrorKind::Unsupported,
                message: "MA-32: deployment `Plugin` is reserved and unsupported in Phase 1"
                    .to_owned(),
                detail: serde_json::Value::Null,
            });
        }
        if d.kernel_api.major != KERNEL_API.major {
            return Err(ModuleError::rejected(format!(
                "MA-32: kernel_api major {} does not match the Kernel's {}",
                d.kernel_api.major, KERNEL_API.major
            )));
        }
        if self.modules.contains_key(&(d.id.clone(), d.version)) {
            return Err(ModuleError::rejected(format!(
                "MA-32: {} {} is already registered",
                d.id, d.version
            )));
        }
        for v in &d.vocabularies {
            match self.vocabularies.get(&v.id) {
                None => {
                    return Err(ModuleError::rejected(format!(
                        "MA-32: vocabulary {} is not registered",
                        v.id
                    )));
                }
                Some(reg) if !v.req.matches(reg.version) => {
                    return Err(ModuleError::rejected(format!(
                        "MA-32: vocabulary {} at {} does not satisfy ^{}",
                        v.id, reg.version, v.req.0
                    )));
                }
                Some(_) => {}
            }
        }
        for role in &d.roles {
            if !factories.has(*role) {
                return Err(ModuleError::rejected(format!(
                    "MA-31: role {role:?} has no factory"
                )));
            }
        }
        for role in factories.roles() {
            if !d.roles.contains(&role) {
                return Err(ModuleError::rejected(format!(
                    "MA-31: factory for {role:?} has no declared role"
                )));
            }
        }
        self.modules.insert((d.id.clone(), d.version), d);
        Ok(())
    }

    /// Registers the Link descriptor supplied by a registered Link Module, keyed by
    /// the `module` it declares, so a descriptor cannot be filed under another
    /// version than its own (D82). A descriptor for an unregistered Module or a
    /// Module without the Link role is refused; Phase 1 also refuses cross-process
    /// links (MA-28, D68).
    pub fn register_link_descriptor(
        &mut self,
        descriptor: LinkDescriptor,
    ) -> Result<(), ModuleError> {
        let module = descriptor.module.clone();
        if descriptor.cross_process {
            return Err(ModuleError::rejected(
                "MA-28: cross_process links are unsupported in v4.0".to_owned(),
            ));
        }
        let has_link_role = self
            .modules
            .get(&(module.id.clone(), module.version))
            .is_some_and(|registered| registered.roles.contains(&Role::Link));
        if !has_link_role {
            return Err(ModuleError::rejected(format!(
                "MA-28: {} {} is not a registered Link Module",
                module.id, module.version
            )));
        }
        if self.link_descriptors.contains_key(&module) {
            return Err(ModuleError::rejected(format!(
                "MA-28: {} {} already has a registered Link descriptor",
                module.id, module.version
            )));
        }
        self.link_descriptors.insert(module, descriptor);
        Ok(())
    }

    /// The registered Vocabulary, if any (MA-34).
    pub fn vocabulary(&self, id: &Namespace) -> Option<&VocabularyDescriptor> {
        self.vocabularies.get(id)
    }

    /// Every registered Module, in `(id, version)` order (MA-32).
    pub fn modules(&self) -> impl Iterator<Item = &ModuleDescriptor> {
        self.modules.values()
    }

    /// The registered Link descriptor selected by a BindingProfile (MA-28, SB-25).
    pub fn link_descriptor(&self, module: &ModuleRef) -> Option<&LinkDescriptor> {
        self.link_descriptors.get(module)
    }

    /// Every registered Link descriptor, keyed by Module id and version (MA-28, SB-25).
    pub fn link_descriptors(&self) -> &BTreeMap<ModuleRef, LinkDescriptor> {
        &self.link_descriptors
    }

    /// The `KeyDecl` for a key, resolved through the Vocabulary that owns its
    /// prefix. The Kernel refuses a key whose prefix belongs to no Vocabulary the
    /// declaring Module declares, naming the prefix, and validates the value's
    /// shape against the `KeyDecl`. It never interprets the meaning.
    ///
    /// Rule: MA-34, SB-2.
    pub fn key_decl(&self, key: &Key) -> Result<&KeyDecl, ModuleError> {
        self.vocabularies
            .values()
            .filter(|v| key.has_prefix(&v.prefix))
            .find_map(|v| v.keys.iter().find(|d| d.key == *key))
            .ok_or_else(|| {
                ModuleError::rejected(format!(
                    "MA-34: key {key} has a prefix no registered Vocabulary owns"
                ))
            })
    }

    /// The compilation a Vocabulary declares for one of its verbs (RS-13a, RS-14).
    pub fn verb(&self, ns: &Namespace, verb: &Ident) -> Option<&VerbDecl> {
        self.vocabularies
            .get(ns)?
            .verbs
            .iter()
            .find(|v| v.verb == *verb)
    }
}
