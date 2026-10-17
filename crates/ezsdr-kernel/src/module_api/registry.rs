//! Explicit registration, compatibility checks and lookup (MA-31…MA-35).

use std::collections::BTreeMap;

use crate::spec::{Ident, Key, KeyDecl, Namespace};

use super::{
    CompileRule, Factories, KERNEL_API, LinkDescriptor, ModuleDescriptor, ModuleError,
    ModuleRef, ModuleRegistry, Role, VerbDecl, VocabularyDescriptor,
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

/// How a declaration is named in a refusal (SB-2a): its spelling, and the segment it
/// numbers, so that two declarations of one spelling are told apart.
fn decl_name(d: &KeyDecl) -> String {
    match d.indexed {
        Some(position) => format!("{} (numbered segment {position})", d.key),
        None => d.key.to_string(),
    }
}

/// The refusal of two declarations that cover one concrete key (SB-2a), naming both.
fn overlap_error(
    a_owner: &Namespace,
    a: &KeyDecl,
    b_owner: &Namespace,
    b: &KeyDecl,
) -> ModuleError {
    ModuleError::rejected(format!(
        "SB-2a: {} of vocabulary {a_owner} and {} of vocabulary {b_owner} cover one concrete key",
        decl_name(a),
        decl_name(b)
    ))
}

impl ModuleRegistry {
    /// An empty registry (MA-32).
    pub fn new() -> ModuleRegistry {
        ModuleRegistry::default()
    }

    /// Registers a Vocabulary, whose keys, event kinds, verbs and checks the Kernel
    /// then enforces without interpreting them (MA-35).
    ///
    /// Rule: MA-32, MA-34, MA-35, SB-2a.
    pub fn register_vocabulary(&mut self, v: VocabularyDescriptor) -> Result<(), ModuleError> {
        if self.vocabularies.contains_key(&v.id) {
            return Err(ModuleError::rejected(format!(
                "MA-32: vocabulary {} is already registered",
                v.id
            )));
        }
        self.check_key_declarations(&v)?;
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
    /// factory no role; or `(id, version)` is already registered.
    ///
    /// Rule: MA-31, MA-32.
    pub fn register(
        &mut self,
        d: ModuleDescriptor,
        factories: Factories,
    ) -> Result<(), ModuleError> {
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

    /// Refuses `v`'s declarations the Kernel cannot resolve (MA-34, SB-2a): a
    /// declaration whose key is not under `v`'s own prefix, which no lookup would
    /// ever reach; a numbered declaration whose position is out of range or points at
    /// one of the Vocabulary prefix's segments, or whose numbered segment is not a
    /// word of `[a-z]+`; and any two declarations that cover one concrete key — one of
    /// `v`'s beside another of `v`'s, or beside a registered Vocabulary's — a
    /// duplicate exact spelling as much as an overlap with a numbered one.
    fn check_key_declarations(&self, v: &VocabularyDescriptor) -> Result<(), ModuleError> {
        // The prefix first, whatever the declaration's form: a key another prefix owns
        // would never be resolved, and the position check below reads the prefix's
        // own segments.
        for decl in &v.keys {
            if !decl.key.has_prefix(&v.prefix) {
                return Err(ModuleError::rejected(format!(
                    "MA-34: {} of vocabulary {} is not under its prefix {}",
                    decl.key, v.id, v.prefix
                )));
            }
        }
        let prefix_segments = v.prefix.as_str().split('.').count();
        // The position and the grammar first, so that two declarations the overlap
        // scan sees are both well formed and their refusal names that alone.
        for decl in &v.keys {
            let Some(position) = decl.indexed else {
                continue;
            };
            let segments: Vec<&str> = decl.key.as_str().split('.').collect();
            let position = usize::from(position);
            if position >= segments.len() {
                return Err(ModuleError::rejected(format!(
                    "SB-2a: {} numbers segment {position}, and the key has {} segments",
                    decl.key,
                    segments.len()
                )));
            }
            if position < prefix_segments {
                return Err(ModuleError::rejected(format!(
                    "SB-2a: {} numbers segment {position}, which is the Vocabulary prefix \
                     {}'s",
                    decl.key, v.prefix
                )));
            }
            if !segments[position].bytes().all(|b| b.is_ascii_lowercase()) {
                return Err(ModuleError::rejected(format!(
                    "SB-2a: {} numbers `{}`, which is not a word of [a-z]+",
                    decl.key, segments[position]
                )));
            }
        }
        for (i, decl) in v.keys.iter().enumerate() {
            for other in v.keys.iter().skip(i + 1) {
                if decl.overlaps(other) {
                    return Err(overlap_error(&v.id, decl, &v.id, other));
                }
            }
            for (id, registered) in &self.vocabularies {
                for other in &registered.keys {
                    if decl.overlaps(other) {
                        return Err(overlap_error(&v.id, decl, id, other));
                    }
                }
            }
        }
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
    /// prefix, or `None` when no registered Vocabulary declares it. A declaration
    /// whose `indexed` is set declares the concrete keys SB-2a describes, and
    /// registration keeps the declaration that covers a key unique, so one pass finds
    /// it. The refusal is the caller's, because its context names it differently:
    /// `validate()` refuses an unknown prefix (`SB-2`, `UnknownKeyPrefix`), while
    /// admission refuses an undeclared update class or kind after its target
    /// resolution (pre-freeze item 3). The Kernel never interprets the key's meaning.
    ///
    /// Rule: MA-34, SB-2, SB-2a.
    pub fn key_decl(&self, key: &Key) -> Option<&KeyDecl> {
        self.vocabularies
            .values()
            .filter(|v| key.has_prefix(&v.prefix))
            .find_map(|v| v.keys.iter().find(|d| d.declares(key)))
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
