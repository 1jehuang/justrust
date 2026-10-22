//! Snapshot-local instance planning with canonical, collision-free native symbols.
//! Local declaration/application IDs are lookup handles, never persistent keys.
use super::{frontend, ir, traits};
use std::sync::Arc;
#[path = "instances/graph_key.rs"]
pub(super) mod graph_key;

const SEMANTICS: &[u8] =
    b"jr-native-normal-drop-v5/linux-x86_64/checked/scalar-root-flags/atomic-canonical-drop/carrier-regions-storage-effects/structured-declaration-paths/complete-trait-applications";

pub(super) struct InstanceIdentity {
    pub symbol: String,
    #[cfg_attr(not(test), allow(dead_code))]
    pub canonical: Arc<[u8]>,
    _reservations: Vec<ir::PlanReservation>,
}

struct Encoder<'a> {
    bytes: Vec<u8>,
    reservations: Vec<ir::PlanReservation>,
    budget: &'a ir::PlanBudget,
    context: &'a frontend::PlanCtx,
}

impl Encoder<'_> {
    fn append(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.budget.charge_work(bytes.len().max(1))?;
        let charge = bytes
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(2 * std::mem::size_of::<ir::PlanReservation>()))
            .ok_or("generic instance key byte limit exceeded")?;
        let reservation = self.budget.reserve_bytes(charge)?;
        self.bytes
            .try_reserve_exact(bytes.len())
            .map_err(|_| "generic instance key allocation limit exceeded")?;
        self.reservations
            .try_reserve_exact(1)
            .map_err(|_| "generic instance key reservation limit exceeded")?;
        self.bytes.extend_from_slice(bytes);
        self.reservations.push(reservation);
        Ok(())
    }

    fn count(&mut self, value: usize) -> Result<(), String> {
        self.append(
            &u64::try_from(value)
                .map_err(|_| "generic instance key length limit exceeded")?
                .to_le_bytes(),
        )
    }

    fn text(&mut self, value: &str) -> Result<(), String> {
        self.count(value.len())?;
        self.append(value.as_bytes())
    }

    fn declaration(&mut self, declaration: ir::DeclId) -> Result<(), String> {
        match self.context.declaration_origin(declaration)? {
            traits::DeclOrigin::Source => self.append(b"S")?,
            traits::DeclOrigin::CoreCatalog { version, item } => {
                self.append(b"C")?;
                self.append(&version.to_le_bytes())?;
                return self.catalog_item(item);
            }
        }
        let path = self.context.persistent_decl_path(declaration)?;
        self.append(&[match path.kind() {
            ir::DeclKind::Function => 0,
            ir::DeclKind::Struct => 1,
            ir::DeclKind::Enum => 2,
            ir::DeclKind::Impl => 3,
            ir::DeclKind::AssociatedFunction => 4,
            ir::DeclKind::Trait => 5,
            ir::DeclKind::TraitItem => 6,
            ir::DeclKind::TraitImpl => 7,
            ir::DeclKind::AssociatedType => 8,
            ir::DeclKind::Const => 9,
            ir::DeclKind::AssociatedConst => 10,
        }])?;
        self.module_path(&path)?;
        match path.item() {
            frontend::PersistentDeclItem::Name(name) => {
                self.append(b"N")?;
                self.text(name)?;
            }
            frontend::PersistentDeclItem::AssociatedType { trait_name, name } => {
                self.append(b"Y")?;
                self.text(trait_name)?;
                self.text(name)?;
            }
            frontend::PersistentDeclItem::Impl => self.append(b"I")?,
            frontend::PersistentDeclItem::Catalog(item) => {
                self.append(b"C")?;
                self.catalog_item(item)?;
            }
        }
        let semantic = self.context.semantic_path(declaration)?;
        if let Some(owner) = &semantic.associated {
            self.append(b"A")?;
            let impl_path = self.context.persistent_decl_path(owner.impl_decl)?;
            self.module_path(&impl_path)?;
            self.declaration(owner.nominal)?;
            let env = self.context.param_env(owner.impl_decl)?;
            self.count(env.arity())?;
            self.pattern(&owner.self_pattern, &env, 0)?;
            match (owner.trait_decl, owner.trait_item) {
                (None, None) => self.append(b"I")?,
                (Some(trait_decl), Some(trait_item)) => {
                    self.append(b"T")?;
                    self.declaration(trait_decl)?;
                    let world = self.context.trait_world()?;
                    let mut application = None;
                    for implementation in world.impl_headers() {
                        self.budget.charge_work(1)?;
                        if implementation.declaration == owner.impl_decl {
                            application = Some(&implementation.trait_ref);
                            break;
                        }
                    }
                    let application =
                        application.ok_or("trait instance key has no checked impl application")?;
                    if application.declaration != trait_decl {
                        return Err("trait instance key disagrees with checked impl owner".into());
                    }
                    self.count(application.args.len())?;
                    for argument in application.args.iter() {
                        self.pattern(argument, &env, 0)?;
                    }
                    if self.context.declaration(trait_item)?.kind != ir::DeclKind::TraitItem {
                        return Err("trait instance key has non-item owner".into());
                    }
                    self.declaration(trait_item)?;
                }
                _ => return Err("trait instance key has incomplete semantic owner".into()),
            }
            self.append(&[match owner.receiver {
                frontend::ReceiverKind::None => 0,
                frontend::ReceiverKind::Value { mutable: false } => 1,
                frontend::ReceiverKind::Value { mutable: true } => 2,
                frontend::ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Shared,
                } => 3,
                frontend::ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Mut,
                } => 4,
            }])
        } else if let Some(owner) = self.context.declaration(declaration)?.owner {
            // Trait items and provided bodies live in the trait's lexical
            // module, not a synthetic child module named after the trait.
            // Their checked owner must therefore be part of persistent identity.
            let owner_info = self.context.declaration(owner)?;
            if !matches!(
                path.kind(),
                ir::DeclKind::TraitItem | ir::DeclKind::AssociatedFunction
            ) || owner_info.kind != ir::DeclKind::Trait
                || owner_info.owner.is_some()
            {
                return Err("instance key has invalid trait declaration owner".into());
            }
            self.append(b"T")?;
            self.declaration(owner)
        } else {
            self.append(b"F")
        }
    }

    /// The checked descriptor borrows normalized atoms from the retained store.
    /// Arena handles, occurrence indices and display strings never enter the wire.
    fn module_path(&mut self, path: &frontend::PersistentDeclPath<'_>) -> Result<(), String> {
        self.append(match path.crate_root() {
            frontend::PersistentCrate::Source => b"S",
            frontend::PersistentCrate::Core => b"C",
        })?;
        let mut count = 0_usize;
        for _ in path.modules() {
            self.budget.charge_work(1)?;
            count = count
                .checked_add(1)
                .ok_or("generic instance module path length limit exceeded")?;
        }
        self.count(count)?;
        for component in path.modules() {
            match component {
                frontend::PersistentModule::Source(name) => {
                    self.append(b"S")?;
                    self.text(name)?;
                }
                frontend::PersistentModule::Catalog(module) => {
                    self.append(b"C")?;
                    self.append(&[match module {
                        super::module_identity::CatalogModule::Root => 0,
                        super::module_identity::CatalogModule::Marker => 1,
                        super::module_identity::CatalogModule::Clone => 2,
                        super::module_identity::CatalogModule::Ops => 3,
                    }])?;
                }
            }
        }
        Ok(())
    }

    fn catalog_item(&mut self, item: traits::CatalogItem) -> Result<(), String> {
        // Explicit wire tags are independent from Rust enum layout and local IDs.
        let tag = match item {
            traits::CatalogItem::Sized => 0,
            traits::CatalogItem::Clone => 1,
            traits::CatalogItem::CloneItem => 2,
            traits::CatalogItem::CloneFromItem => 3,
            traits::CatalogItem::CloneFromDefault => 4,
            traits::CatalogItem::Copy => 5,
            traits::CatalogItem::Drop => 6,
            traits::CatalogItem::DropItem => 7,
            traits::CatalogItem::Recipe(descriptor) => {
                self.append(&[8])?;
                return self.recipe(descriptor);
            }
        };
        self.append(&[tag])
    }

    fn recipe(&mut self, descriptor: traits::RecipeDescriptor) -> Result<(), String> {
        match descriptor {
            traits::RecipeDescriptor::CloneInt(integer) => {
                self.append(&[0])?;
                self.text(integer.name())
            }
            traits::RecipeDescriptor::CloneFloat(float) => {
                self.append(&[7])?;
                self.text(float.name())
            }
            traits::RecipeDescriptor::CloneBool => self.append(&[1]),
            traits::RecipeDescriptor::CloneChar => self.append(&[8]),
            traits::RecipeDescriptor::CloneUnit => self.append(&[2]),
            traits::RecipeDescriptor::CloneNever => self.append(&[3]),
            traits::RecipeDescriptor::CloneTuple { arity } => {
                self.append(&[4])?;
                self.count(usize::from(arity))
            }
            traits::RecipeDescriptor::CloneArray { len } => {
                self.append(&[5])?;
                self.count(usize::from(len))
            }
            traits::RecipeDescriptor::CloneSharedRef => self.append(&[6]),
        }
    }

    /// Pattern parameters are alpha ordinals in the authoritative impl scope,
    /// never process-local binder IDs. Concrete instance arguments use `ty`.
    fn pattern(&mut self, ty: &ir::Ty, env: &ir::ParamEnv, depth: usize) -> Result<(), String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        match ty {
            ir::Ty::Opaque(opaque) => {
                self.append(b"O")?;
                self.declaration(opaque.owner())?;
                self.count(opaque.site() as usize)?;
                self.count(opaque.owner_args().len())?;
                for ty in opaque.owner_args() {
                    self.pattern(ty, env, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Closure(closure) => {
                self.append(b"C")?;
                self.declaration(closure.owner())?;
                self.count(closure.site() as usize)?;
                self.declaration(closure.provider())?;
                self.count(closure.owner_args().len())?;
                self.count(closure.params().len())?;
                self.count(closure.captures().len())?;
                self.append(&[closure.kind() as u8])?;
                for ty in closure.types() {
                    self.pattern(ty, env, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::FnItem(item) => {
                self.append(b"F")?;
                self.declaration(item.declaration)?;
                self.count(item.type_args.len())?;
                for ty in item.type_args.iter() {
                    self.pattern(ty, env, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Param(param) => {
                if !env.contains(*param) {
                    return Err("associated instance pattern has foreign parameter".into());
                }
                self.append(b"p")?;
                self.count(usize::from(param.index))
            }
            ir::Ty::Int(integer) => {
                self.append(b"i")?;
                self.text(integer.name())
            }
            ir::Ty::Float(float) => {
                self.append(b"f")?;
                self.text(float.name())
            }
            ir::Ty::Bool => self.append(b"b"),
            ir::Ty::Char => self.append(b"c"),
            ir::Ty::Str => self.append(b"z"),
            ir::Ty::Unit => self.append(b"u"),
            ir::Ty::Tuple(fields) => {
                self.append(b"t")?;
                self.count(fields.len())?;
                for field in fields.iter() {
                    self.pattern(field, env, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Array { element, len } => {
                self.append(b"a")?;
                self.count(*len)?;
                self.pattern(element, env, depth + 1)
            }
            ir::Ty::Slice { element } => {
                self.append(b"l")?;
                self.pattern(element, env, depth + 1)
            }
            ir::Ty::Struct(structure) => {
                self.append(b"s")?;
                self.declaration(structure.def().source_decl)?;
                self.count(structure.def().type_args.len())?;
                for argument in structure.def().type_args.iter() {
                    self.pattern(argument, env, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Enum(enumeration) => {
                self.append(b"e")?;
                self.declaration(enumeration.def().source_decl)?;
                self.count(enumeration.def().type_args.len())?;
                for argument in enumeration.def().type_args.iter() {
                    self.pattern(argument, env, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Ref { kind, referent } => {
                self.append(&[match kind {
                    ir::BorrowKind::Shared => b'r',
                    ir::BorrowKind::Mut => b'm',
                }])?;
                self.pattern(referent, env, depth + 1)
            }
            ir::Ty::RawPtr { kind, pointee } => {
                self.append(match kind {
                    ir::BorrowKind::Shared => b"P",
                    ir::BorrowKind::Mut => b"M",
                })?;
                self.pattern(pointee, env, depth + 1)
            }
            // --- trait objects (snail): `dyn Trait` identity tag b'D' ---
            ir::Ty::Dyn(object) => {
                self.append(b"D")?;
                self.declaration(object.principal)?;
                self.count(object.args.len())?;
                for argument in object.args.iter() {
                    self.pattern(argument, env, depth + 1)?;
                }
                self.count(object.associated_types.len())?;
                for (item, value) in object.associated_types.iter() {
                    self.declaration(*item)?;
                    self.pattern(value, env, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Never => Err("unsupported associated instance self pattern".into()),
            ir::Ty::AssociatedType(_) => {
                Err("associated instance pattern contains unresolved associated type".into())
            }
        }
    }

    fn ty(&mut self, ty: &ir::Ty, depth: usize) -> Result<(), String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        match ty {
            ir::Ty::Opaque(opaque) => {
                self.append(b"O")?;
                self.declaration(opaque.owner())?;
                self.count(opaque.site() as usize)?;
                self.count(opaque.owner_args().len())?;
                for ty in opaque.owner_args() {
                    self.ty(ty, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Closure(closure) => {
                self.append(b"C")?;
                self.declaration(closure.owner())?;
                self.count(closure.site() as usize)?;
                self.declaration(closure.provider())?;
                self.count(closure.owner_args().len())?;
                self.count(closure.params().len())?;
                self.count(closure.captures().len())?;
                self.append(&[closure.kind() as u8])?;
                for ty in closure.types() {
                    self.ty(ty, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::FnItem(item) => {
                self.append(b"F")?;
                self.declaration(item.declaration)?;
                self.count(item.type_args.len())?;
                for ty in item.type_args.iter() {
                    self.ty(ty, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Int(integer) => {
                self.append(b"i")?;
                self.text(integer.name())
            }
            ir::Ty::Float(float) => {
                self.append(b"f")?;
                self.text(float.name())
            }
            ir::Ty::Bool => self.append(b"b"),
            ir::Ty::Char => self.append(b"c"),
            ir::Ty::Str => self.append(b"z"),
            ir::Ty::Unit => self.append(b"u"),
            ir::Ty::Never => self.append(b"n"),
            ir::Ty::Tuple(fields) => {
                self.append(b"t")?;
                self.count(fields.len())?;
                for field in fields.iter() {
                    self.ty(field, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Array { element, len } => {
                self.append(b"a")?;
                self.count(*len)?;
                // Zero-sized arrays still contribute their exact element identity.
                self.ty(element, depth + 1)
            }
            ir::Ty::Slice { element } => {
                self.append(b"l")?;
                self.ty(element, depth + 1)
            }
            ir::Ty::Struct(structure) => {
                self.append(b"s")?;
                self.declaration(structure.def().source_decl)?;
                self.count(structure.def().type_args.len())?;
                for argument in structure.def().type_args.iter() {
                    self.ty(argument, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Enum(enumeration) => {
                self.append(b"e")?;
                self.declaration(enumeration.def().source_decl)?;
                self.count(enumeration.def().type_args.len())?;
                for argument in enumeration.def().type_args.iter() {
                    self.ty(argument, depth + 1)?;
                }
                Ok(())
            }
            // --- trait objects (snail): `dyn Trait` identity tag b'D' ---
            ir::Ty::Dyn(object) => {
                self.append(b"D")?;
                self.declaration(object.principal)?;
                self.count(object.args.len())?;
                for argument in object.args.iter() {
                    self.ty(argument, depth + 1)?;
                }
                self.count(object.associated_types.len())?;
                for (item, value) in object.associated_types.iter() {
                    self.declaration(*item)?;
                    self.ty(value, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::Param(_) => Err("generic instance key contains unresolved parameter".into()),
            ir::Ty::AssociatedType(_) => {
                Err("generic instance key contains unresolved associated type".into())
            }
            ir::Ty::Ref { kind, referent } => {
                self.append(match kind {
                    ir::BorrowKind::Shared => b"rs",
                    ir::BorrowKind::Mut => b"rm",
                })?;
                self.ty(referent, depth + 1)
            }
            ir::Ty::RawPtr { kind, pointee } => {
                self.append(match kind {
                    ir::BorrowKind::Shared => b"P",
                    ir::BorrowKind::Mut => b"M",
                })?;
                self.ty(pointee, depth + 1)
            }
        }
    }
}

/// The exact symbol `Plan::symbol` returns for `target`, derived from the
/// target alone. Lets an object reference callees without reading the
/// whole-program plan; the link step still rejects any unplanned symbol.
pub(super) fn symbol_of(
    target: &ir::CallTarget,
    context: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<String, String> {
    if context.is_entry_decl(target.declaration)? && target.type_args.is_empty() {
        return Ok("main".into());
    }
    Ok(identity(target, context, budget)?.symbol)
}

pub(super) fn identity(
    target: &ir::CallTarget,
    context: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<InstanceIdentity, String> {
    let mut encoder = Encoder {
        bytes: Vec::new(),
        reservations: Vec::new(),
        budget,
        context,
    };
    encoder.append(SEMANTICS)?;
    encoder.declaration(target.declaration)?;
    encoder.count(target.type_args.len())?;
    for argument in target.type_args.iter() {
        encoder.ty(argument, 0)?;
    }
    let symbol_bytes = encoder
        .bytes
        .len()
        .checked_mul(2)
        .and_then(|n| n.checked_add(4))
        .ok_or("generic instance symbol byte limit exceeded")?;
    let symbol_reservation = budget.reserve_bytes(symbol_bytes)?;
    let canonical_reservation = budget.reserve_bytes(
        encoder
            .bytes
            .len()
            .checked_add(2 * std::mem::size_of::<usize>())
            .ok_or("generic canonical key byte limit exceeded")?,
    )?;
    budget.charge_work(encoder.bytes.len())?;
    let mut symbol = String::new();
    symbol
        .try_reserve_exact(symbol_bytes)
        .map_err(|_| "generic instance symbol allocation limit exceeded")?;
    // '$' cannot occur in an admitted source identifier, avoiding source collision.
    symbol.push_str("$jr$");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in &encoder.bytes {
        symbol.push(char::from(HEX[usize::from(byte >> 4)]));
        symbol.push(char::from(HEX[usize::from(byte & 15)]));
    }
    let bookkeeping = budget.reserve_bytes(6 * std::mem::size_of::<ir::PlanReservation>())?;
    encoder
        .reservations
        .try_reserve_exact(3)
        .map_err(|_| "generic instance reservation allocation limit exceeded")?;
    encoder.reservations.push(symbol_reservation);
    encoder.reservations.push(canonical_reservation);
    encoder.reservations.push(bookkeeping);
    Ok(InstanceIdentity {
        symbol,
        canonical: Arc::from(encoder.bytes),
        _reservations: encoder.reservations,
    })
}

#[cfg(test)]
use std::collections::HashMap;

#[cfg(test)]
fn charge_target_lookup(target: &ir::CallTarget, budget: &ir::PlanBudget) -> Result<(), String> {
    fn walk(value: &ir::Ty, depth: usize, budget: &ir::PlanBudget) -> Result<(), String> {
        budget.check_depth(depth)?;
        // Cover the bounded group of Hash/Eq operations following this walk.
        budget.charge_work(5)?;
        match value {
            ir::Ty::Opaque(opaque) => {
                for ty in opaque.types() {
                    walk(ty, depth + 1, budget)?;
                }
            }
            ir::Ty::Closure(closure) => {
                for ty in closure.types() {
                    walk(ty, depth + 1, budget)?;
                }
            }
            ir::Ty::FnItem(item) => {
                for ty in item.type_args.iter() {
                    walk(ty, depth + 1, budget)?;
                }
            }
            ir::Ty::Tuple(fields) => {
                for field in fields.iter() {
                    walk(field, depth + 1, budget)?;
                }
            }
            ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                walk(element, depth + 1, budget)?
            }
            ir::Ty::Dyn(object) => {
                for value in object.types() {
                    walk(value, depth + 1, budget)?;
                }
            }
            ir::Ty::Struct(_)
            | ir::Ty::Enum(_)
            | ir::Ty::Int(_)
            | ir::Ty::Float(_)
            | ir::Ty::Char
            | ir::Ty::Str
            | ir::Ty::Bool
            | ir::Ty::Unit
            | ir::Ty::Never => {}
            ir::Ty::Ref { referent, .. }
            | ir::Ty::RawPtr {
                pointee: referent, ..
            } => walk(referent, depth + 1, budget)?,
            ir::Ty::Param(_) | ir::Ty::AssociatedType(_) => {
                return Err("generic instance has non-closed argument".into());
            }
        }
        Ok(())
    }
    budget.charge_work(1)?;
    for argument in target.type_args.iter() {
        walk(argument, 0, budget)?;
    }
    Ok(())
}

/// Two raw targets with one canonical key may share a planned instance only
/// when both validate under their own closed instance facts and the validated
/// trait world proves their ordered arguments equal (canonical declaration and
/// arguments, both representations validated). Anything else stays a
/// collision error: differing authorization never merges.
pub(super) fn admit_representation_alias(
    previous: &ir::CallTarget,
    next: &ir::CallTarget,
    context: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    if previous.declaration != next.declaration || previous.type_args.len() != next.type_args.len()
    {
        return Ok(false);
    }
    let previous_facts = context.instance_facts(previous)?;
    let next_facts = context.instance_facts(next)?;
    for facts in [&previous_facts, &next_facts] {
        for ty in previous.type_args.iter().chain(next.type_args.iter()) {
            budget.charge_work(1)?;
            facts
                .validate_ty(ty)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
        }
        if !facts
            .world()
            .types_equal(&previous.type_args, &next.type_args)
            .map_err(|e| e.into_legacy_diagnostic(budget))?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
pub(super) struct Instance {
    pub target: ir::CallTarget,
    pub source_index: usize,
    pub identity: InstanceIdentity,
}

#[cfg(test)]
pub(super) struct Plan {
    pub entries: Vec<Instance>,
    cleanup_targets: Vec<ir::CallTarget>,
    lookup: HashMap<ir::CallTarget, usize>,
    _reservations: Vec<ir::PlanReservation>,
    budget: ir::PlanBudget,
}

#[cfg(test)]
impl Plan {
    pub fn validate_cleanup_dependencies(
        &self,
        function: &super::cfg::Function,
    ) -> Result<(), String> {
        for block in &function.blocks {
            self.budget.charge_work(1)?;
            if let super::cfg::Terminator::Drop { cleanup, .. } = &block.terminator.value {
                for target in cleanup.destructor().targets() {
                    let target = &target;
                    self.budget.charge_work(self.cleanup_targets.len())?;
                    if !self.cleanup_targets.contains(target) {
                        return Err("cleanup target lacks sealed implicit instance demand".into());
                    }
                    charge_target_lookup(target, &self.budget)?;
                    if !self.lookup.contains_key(target) {
                        return Err("cleanup target is absent from executable plan".into());
                    }
                }
            }
        }
        Ok(())
    }
    /// Plan slot of `target`, if it is a planned instance.
    pub fn slot(&self, target: &ir::CallTarget) -> Option<usize> {
        self.lookup.get(target).copied()
    }
    pub fn symbol(&self, target: &ir::CallTarget) -> Result<String, String> {
        charge_target_lookup(target, &self.budget)?;
        let index = self
            .lookup
            .get(target)
            .ok_or("call target is not in the sealed generic instance plan")?;
        let symbol = &self.entries[*index].identity.symbol;
        self.budget.charge_work(symbol.len())?;
        // Also cover the retained Object.calls slot and its geometric growth
        // peak before the backend appends the returned owning string.
        self.budget.retain_bytes(
            symbol
                .len()
                .checked_add(4 * std::mem::size_of::<(usize, String)>())
                .ok_or("generic relocation metadata byte limit exceeded")?,
        )?;
        let mut copy = String::new();
        copy.try_reserve_exact(symbol.len())
            .map_err(|_| "generic relocation symbol allocation limit exceeded")?;
        copy.push_str(symbol);
        Ok(copy)
    }
}

/// The direct executable demands of one closed instance, read from its own
/// universally checked template body alone: explicit direct callees, trait
/// object vtable slots and implicit cleanup destructors. Opaque trait-item
/// calls are deferred to operation 140 and not listed here.
#[derive(Default)]
pub(super) struct DirectDemands {
    /// Every demanded target in first-occurrence order, deduplicated.
    pub targets: Vec<ir::CallTarget>,
    /// The subset demanded by implicit normal-drop cleanup.
    pub cleanup: Vec<ir::CallTarget>,
}
impl DirectDemands {
    /// Every implicit cleanup destructor in `function` must be one of this
    /// instance's own sealed cleanup demands, so BUILD links it.
    pub fn validate_cleanup(
        &self,
        function: &super::cfg::Function,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        for block in &function.blocks {
            budget.charge_work(1)?;
            if let super::cfg::Terminator::Drop { cleanup, .. } = &block.terminator.value {
                for target in cleanup.destructor().targets() {
                    budget.charge_work(self.cleanup.len().max(1))?;
                    if !self.cleanup.contains(&target) {
                        return Err("cleanup target lacks sealed implicit instance demand".into());
                    }
                }
            }
        }
        Ok(())
    }
    fn call(&mut self, target: ir::CallTarget, budget: &ir::PlanBudget) -> Result<(), String> {
        budget.charge_work(self.targets.len().max(1))?;
        if !self.targets.contains(&target) {
            budget.retain_bytes(2 * std::mem::size_of::<ir::CallTarget>())?;
            self.targets
                .try_reserve(1)
                .map_err(|_| "instance demand allocation failed")?;
            self.targets.push(target);
        }
        Ok(())
    }
    fn cleanup(&mut self, target: ir::CallTarget, budget: &ir::PlanBudget) -> Result<(), String> {
        budget.charge_work(self.cleanup.len().max(1))?;
        if !self.cleanup.contains(&target) {
            budget.retain_bytes(2 * std::mem::size_of::<ir::CallTarget>())?;
            self.cleanup
                .try_reserve(1)
                .map_err(|_| "cleanup demand allocation failed")?;
            self.cleanup.push(target.clone());
        }
        self.call(target, budget)
    }
}

pub(super) fn direct_demands(
    target: &ir::CallTarget,
    source: &ir::CheckedFunction,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<crate::engine::EngineAuthority>,
) -> Result<DirectDemands, String> {
    let mut demands = DirectDemands::default();
    let schema = context.function_schema(target.declaration)?;
    let env = schema.env;
    let closed_facts = context.instance_facts(&target)?;
    // Implicit cleanup is part of the same closed demand frontier as an
    // explicit call. Do not rely on nongeneric source-root enumeration.
    if closed_facts.has_normal_drop_impls() {
        for ty in source
            .locals
            .iter()
            .map(|local| &local.ty)
            .chain(source.exprs.iter().map(|expr| &expr.ty))
            .chain(std::iter::once(&source.result))
        {
            budget.charge_work(1)?;
            // Checked primitive/reference/pointer outer forms cannot acquire
            // owning Drop through substitution. Nominal and aggregate forms
            // still require the full closed canonical descriptor below.
            if matches!(
                ty,
                ir::Ty::Int(_)
                    | ir::Ty::Float(_)
                    | ir::Ty::Bool
                    | ir::Ty::Char
                    | ir::Ty::Unit
                    | ir::Ty::Never
                    | ir::Ty::Ref { .. }
                    | ir::Ty::RawPtr { .. }
            ) {
                continue;
            }
            let ty = super::nominal_realization::substitution::substitute_ty(
                ty,
                &env,
                &target.type_args,
                ir::FactsCtx::from_validated(&closed_facts),
                context,
                budget,
                engine,
            )?;
            match closed_facts
                .normal_drop_plan(&ty)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                traits::NormalDropPlan::Destructor(destructor) => {
                    for target in destructor.targets() {
                        demands.cleanup(target, budget)?;
                    }
                }
                traits::NormalDropPlan::Trivial => {}
                traits::NormalDropPlan::Unresolved => {
                    return Err("unsupported: unresolved closed Drop demand".into());
                }
            }
        }
    }
    for (expression_index, expression) in source.exprs.iter().enumerate() {
        budget.charge_work(1)?;
        // --- trait objects (snail): each unsizing site demands every
        // vtable slot implementation for its exact substituted referent.
        if let ir::ExprKind::RefToDyn { reference } = &expression.kind {
            let ir::Ty::Ref { referent, .. } = &expression.ty else {
                return Err("trait object unsizing result must be a reference".into());
            };
            let ir::Ty::Dyn(object) = referent.as_ref() else {
                return Err("trait object unsizing result must be a trait object".into());
            };
            let operand = source
                .exprs
                .get(reference.0)
                .ok_or("trait object unsizing operand outside arena")?;
            let concrete = super::nominal_realization::substitution::substitute_ty(
                &operand.ty,
                &env,
                &target.type_args,
                ir::FactsCtx::from_validated(&closed_facts),
                context,
                budget,
                engine,
            )?;
            let ir::Ty::Ref { referent, .. } = &concrete else {
                return Err("trait object unsizing operand must be a reference".into());
            };
            for slot in closed_facts
                .object_vtable_targets(referent, object)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                demands.call(slot, budget)?;
            }
            if closed_facts.has_normal_drop_impls()
                && let traits::NormalDropPlan::Destructor(destructor) = closed_facts
                    .normal_drop_plan(referent)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                for target in destructor.targets() {
                    demands.call(target, budget)?;
                }
            }
        }
        if let ir::ExprKind::Call {
            target: ir::Callee::Virtual(_),
            ..
        } = &expression.kind
        {
            // Dispatch targets are demanded at the unsizing sites above.
            continue;
        }
        // --- end trait objects ---
        if let ir::ExprKind::Call { target: callee, .. } = &expression.kind {
            if let ir::Callee::TraitItem(call) = callee
                && matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_))
            {
                // The immutable original TraitItem expression is the deferred
                // edge. OBJECT reads that exact occurrence and operation 140
                // replays its publication and hidden-representation authority.
                // No copied side inventory or physical target is queued here.
                continue;
            }
            let resolved = super::nominal_realization::substitution::substitute_callee(
                callee,
                &env,
                &target.type_args,
                &closed_facts,
                context,
                budget,
                engine,
            )?;
            let next = match resolved {
                ir::Callee::Direct(next) => next,
                // Builtin function item dispatch executes the item itself.
                ir::Callee::Untupled(call) => {
                    closed_facts
                        .service()
                        .untupled_target(&call, &closed_facts)
                        .map_err(|e| e.into_legacy_diagnostic(budget))?
                        .target
                }
                _ => return Err("unresolved trait callee in concrete instance frontier".into()),
            };
            let _ = expression_index;
            demands.call(next, budget)?;
        }
    }
    Ok(demands)
}

#[cfg(test)]
pub(super) fn build(
    context: &mut frontend::PlanCtx,
    sources: &[Arc<ir::CheckedFunction>],
    budget: &ir::PlanBudget,
    engine: Option<crate::engine::EngineAuthority>,
) -> Result<Plan, String> {
    let mut declarations = HashMap::new();
    let mut entries = Vec::new();
    let mut seen = HashMap::new();
    let mut depths = Vec::new();
    let mut reservations = Vec::new();
    let mut canonical_keys = HashMap::new();
    let mut cleanup_targets = Vec::new();
    let registry_reservation = budget.reserve_bytes(
        context
            .callable_count()
            .checked_mul(4 * (std::mem::size_of::<ir::DeclId>() + std::mem::size_of::<usize>()))
            .ok_or("generic declaration lookup byte limit exceeded")?,
    )?;
    declarations
        .try_reserve(context.callable_count())
        .map_err(|_| "generic declaration lookup allocation limit exceeded")?;
    for index in 0..context.callable_count() {
        budget.charge_work(1)?;
        let declaration = context.callable_decl(index)?;
        if declarations.insert(declaration, index).is_some() {
            return Err("duplicate function declaration identity".into());
        }
    }
    reservations.push(registry_reservation);

    struct Queue<'a> {
        declarations: &'a HashMap<ir::DeclId, usize>,
        entries: &'a mut Vec<Instance>,
        seen: &'a mut HashMap<ir::CallTarget, usize>,
        depths: &'a mut Vec<usize>,
        canonical_keys: &'a mut HashMap<Arc<[u8]>, ir::CallTarget>,
        reservations: &'a mut Vec<ir::PlanReservation>,
        aliases: &'a mut Vec<(ir::CallTarget, ir::CallTarget)>,
    }
    impl Queue<'_> {
        fn add(
            &mut self,
            target: ir::CallTarget,
            depth: usize,
            context: &mut frontend::PlanCtx,
            budget: &ir::PlanBudget,
        ) -> Result<(), String> {
            // A cached target or canonical nominal app is not admission authority.
            // Validate all exact instance metadata before residual projection
            // rejection or any recursive/cache-hit lookup.
            context.instance_facts(&target)?;
            charge_target_lookup(&target, budget)?;
            // Exact recursive instances are already finite, regardless of call depth.
            if self.seen.contains_key(&target) {
                return Ok(());
            }
            budget.check_depth(depth)?;
            let source_index = *self
                .declarations
                .get(&target.declaration)
                .ok_or("generic call does not resolve to a function declaration")?;
            // Entry predicates are proved even if the body has no calls.
            // Caller-side discharge remains separate in substitute_callee.
            budget.charge_application()?;
            budget.charge_nodes(1)?;
            let reservation = budget.reserve_bytes(
                4 * (std::mem::size_of::<Instance>()
                    + std::mem::size_of::<ir::CallTarget>()
                    + std::mem::size_of::<usize>()
                    + std::mem::size_of::<ir::PlanReservation>()),
            )?;
            self.entries
                .try_reserve_exact(1)
                .map_err(|_| "generic instance frontier allocation limit exceeded")?;
            self.depths
                .try_reserve_exact(1)
                .map_err(|_| "generic instance depth allocation limit exceeded")?;
            self.seen
                .try_reserve(1)
                .map_err(|_| "generic instance lookup allocation limit exceeded")?;
            self.canonical_keys
                .try_reserve(1)
                .map_err(|_| "generic canonical identity allocation limit exceeded")?;
            self.reservations
                .try_reserve_exact(1)
                .map_err(|_| "generic plan reservation allocation limit exceeded")?;
            let mut identity = identity(&target, context, budget)?;
            if context.is_entry_decl(target.declaration)? && target.type_args.is_empty() {
                identity.symbol.clear();
                identity.symbol.push_str("main");
            }
            budget.charge_work(
                identity
                    .canonical
                    .len()
                    .checked_mul(3)
                    .ok_or("generic canonical lookup work limit exceeded")?,
            )?;
            if let Some(previous) = self.canonical_keys.get(&identity.canonical)
                && previous != &target
                && admit_representation_alias(previous, &target, context, budget)?
            {
                // Same canonical instance reached through a second, separately
                // validated nominal representation. Seal it as a lookup alias of
                // the one planned instance; no second body is planned.
                budget.retain_bytes(2 * std::mem::size_of::<ir::CallTarget>())?;
                self.aliases
                    .try_reserve(1)
                    .map_err(|_| "generic instance alias allocation limit exceeded")?;
                self.aliases.push((target.clone(), previous.clone()));
                let index = *self
                    .seen
                    .get(previous)
                    .ok_or("canonical instance alias names an unplanned target")?;
                self.seen.insert(target, index);
                return Ok(());
            }
            if let Some(previous) = self.canonical_keys.get(&identity.canonical)
                && previous != &target
            {
                let describe = |ty: &ir::Ty| {
                    let mut current = ty;
                    let mut prefix = String::new();
                    for _ in 0..8 {
                        match current {
                            ir::Ty::Ref { kind, referent } => {
                                prefix.push_str(&format!("Ref({kind:?})->"));
                                current = referent;
                            }
                            ir::Ty::RawPtr { kind, pointee } => {
                                prefix.push_str(&format!("Ptr({kind:?})->"));
                                current = pointee;
                            }
                            _ => break,
                        }
                    }
                    match current {
                        ir::Ty::Struct(value) => format!(
                            "{prefix}Struct(decl={:?},id={:?},args={})",
                            value.def().source_decl,
                            value.id(),
                            value.def().type_args.len()
                        ),
                        ir::Ty::Enum(value) => format!(
                            "{prefix}Enum(decl={:?},id={:?},args={})",
                            value.def().source_decl,
                            value.id(),
                            value.def().type_args.len()
                        ),
                        _ => format!("{prefix}kind={:?}", std::mem::discriminant(current)),
                    }
                };
                return Err(format!(
                    "generic canonical instance identity collision: previous={:?} args={:?}; next={:?} args={:?}",
                    previous.declaration,
                    previous.type_args.iter().map(&describe).collect::<Vec<_>>(),
                    target.declaration,
                    target.type_args.iter().map(&describe).collect::<Vec<_>>()
                ));
            }
            self.canonical_keys
                .insert(identity.canonical.clone(), target.clone());
            self.seen.insert(target.clone(), self.entries.len());
            self.entries.push(Instance {
                target,
                source_index,
                identity,
            });
            self.depths.push(depth);
            self.reservations.push(reservation);
            Ok(())
        }
    }
    let mut aliases = Vec::new();
    let mut queue = Queue {
        declarations: &declarations,
        entries: &mut entries,
        seen: &mut seen,
        depths: &mut depths,
        canonical_keys: &mut canonical_keys,
        reservations: &mut reservations,
        aliases: &mut aliases,
    };
    // Preserve legacy source-order telemetry for nongeneric roots. These indices
    // are local slots only. Canonical identity never contains this ordering.
    for index in 0..context.callable_count() {
        let declaration = context.callable_decl(index)?;
        let source = match context.callable_provider(declaration)? {
            frontend::CallableProvider::Source { .. } => true,
            frontend::CallableProvider::Recipe { .. } => false,
            frontend::CallableProvider::UnavailableProvided { .. } => {
                return Err(
                    "unsupported: unavailable provided body in executable instance inventory"
                        .into(),
                );
            }
        };
        // Nongeneric native prelude bodies are library code: still checked
        // universally, but instantiated only when reachable from user roots.
        if source
            && !context.callable_env(declaration)?.is_generic()
            && !context.is_prelude_decl(declaration)?
        {
            queue.add(
                ir::CallTarget {
                    declaration,
                    type_args: Arc::from([]),
                },
                0,
                context,
                budget,
            )?;
        }
    }
    let roots = queue.entries.len();
    let mut cursor = 0;
    while cursor < queue.entries.len() {
        budget.charge_work(1)?;
        let source_index = queue.entries[cursor].source_index;
        let target = queue.entries[cursor].target.clone();
        let depth = queue.depths[cursor];
        let source = sources
            .get(source_index)
            .ok_or("missing universally checked declaration body")?;
        let demands = direct_demands(&target, source, context, budget, engine)?;
        for next in demands.cleanup {
            budget.charge_work(cleanup_targets.len())?;
            if !cleanup_targets.contains(&next) {
                budget.retain_bytes(2 * std::mem::size_of::<ir::CallTarget>())?;
                cleanup_targets
                    .try_reserve(1)
                    .map_err(|_| "cleanup demand allocation failed")?;
                cleanup_targets.push(next);
            }
        }
        for next in demands.targets {
            queue
                .add(next, depth + 1, context, budget)
                .map_err(|error| format!("{error}; instance caller={:?}", target.declaration))?;
        }
        cursor += 1;
    }
    // Roots retain existing source indices. The additional frontier is sorted by
    // full structural keys, not hash values or concurrent discovery order.
    let additional = &mut entries[roots..];
    let max_key = additional
        .iter()
        .map(|item| item.identity.canonical.len())
        .max()
        .unwrap_or(0);
    let levels = usize::BITS as usize - additional.len().leading_zeros() as usize;
    let comparisons = additional
        .len()
        .checked_mul(levels)
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_mul(max_key.max(1)))
        .ok_or("generic canonical ordering work limit exceeded")?;
    budget.charge_work(comparisons)?;
    additional
        .sort_unstable_by(|left, right| left.identity.canonical.cmp(&right.identity.canonical));
    let mut lookup = HashMap::new();
    let lookup_reservation = budget.reserve_bytes(
        entries
            .len()
            .checked_mul(4 * (std::mem::size_of::<ir::CallTarget>() + std::mem::size_of::<usize>()))
            .ok_or("generic sealed lookup byte limit exceeded")?,
    )?;
    lookup
        .try_reserve(entries.len())
        .map_err(|_| "generic sealed lookup allocation limit exceeded")?;
    for (index, instance) in entries.iter().enumerate() {
        charge_target_lookup(&instance.target, budget)?;
        lookup.insert(instance.target.clone(), index);
    }
    lookup
        .try_reserve(aliases.len())
        .map_err(|_| "generic sealed lookup allocation limit exceeded")?;
    for (alias, canonical) in &aliases {
        charge_target_lookup(alias, budget)?;
        let index = *lookup
            .get(canonical)
            .ok_or("canonical instance alias target is absent from sealed plan")?;
        if lookup.insert(alias.clone(), index).is_some() {
            return Err("canonical instance alias duplicates a planned target".into());
        }
    }
    reservations.push(lookup_reservation);
    Ok(Plan {
        entries,
        cleanup_targets,
        lookup,
        _reservations: reservations,
        budget: budget.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Inventory selection only. Production resolution and entry selection use
    // checked typed APIs, never these persistence atom descriptors.
    fn named(context: &frontend::PlanCtx, declaration: ir::DeclId, name: &str) -> bool {
        matches!(
            context.persistent_decl_path(declaration).unwrap().item(),
            frontend::PersistentDeclItem::Name(atom) if atom == name
        )
    }

    fn source_keys(source: &str) -> Vec<(Vec<u8>, String)> {
        let program = frontend::parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let mut keys = Vec::new();
        for index in 0..context.callable_count() {
            let declaration = context.callable_decl(index).unwrap();
            if !matches!(
                context.callable_provider(declaration).unwrap(),
                frontend::CallableProvider::Source { .. }
            ) {
                continue;
            }
            let key = identity(
                &ir::CallTarget {
                    declaration,
                    type_args: Arc::from([]),
                },
                &context,
                &budget,
            )
            .unwrap();
            assert!(key.canonical.starts_with(SEMANTICS));
            keys.push((key.canonical.to_vec(), key.symbol));
        }
        keys.sort();
        keys
    }

    #[test]
    fn module_keys_ignore_arena_allocation_and_name_occurrence_order() {
        let first = source_keys(
            "mod outer{pub mod left{pub fn item()->i32{1}}pub mod right{pub fn item()->i32{2}}}fn main(){}",
        );
        let reordered = source_keys(
            "fn main(){}mod outer{pub mod right{pub fn item()->i32{let unused=2;unused}}pub mod left{pub fn item()->i32{let padding=1;padding}}}",
        );
        assert_eq!(first.len(), 3);
        assert_eq!(first, reordered);
        assert!(first.windows(2).all(|pair| pair[0].0 != pair[1].0));
    }

    #[test]
    fn module_keys_preserve_segment_boundaries_and_item_atoms() {
        let split_left = source_keys("mod ab{pub mod c{pub fn item()->i32{1}}}fn main(){}");
        let split_right = source_keys("mod a{pub mod bc{pub fn item()->i32{1}}}fn main(){}");
        let renamed = source_keys("mod ab{pub mod c{pub fn other()->i32{1}}}fn main(){}");
        assert_eq!(split_left.len(), 2);
        assert_eq!(split_right.len(), 2);
        assert_eq!(renamed.len(), 2);
        assert_ne!(split_left, split_right);
        assert_ne!(split_left, renamed);
    }

    #[test]
    fn entry_selection_uses_checked_source_root_not_item_spelling_alone() {
        let program = frontend::parse("mod nested{pub fn main(){}}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let mut entries = 0;
        let mut nested = 0;
        for index in 0..context.callable_count() {
            let declaration = context.callable_decl(index).unwrap();
            if !matches!(
                context.callable_provider(declaration).unwrap(),
                frontend::CallableProvider::Source { .. }
            ) {
                continue;
            }
            let path = context.persistent_decl_path(declaration).unwrap();
            assert!(matches!(
                path.crate_root(),
                frontend::PersistentCrate::Source
            ));
            assert!(matches!(
                path.item(),
                frontend::PersistentDeclItem::Name("main")
            ));
            if context.is_entry_decl(declaration).unwrap() {
                assert_eq!(path.modules().count(), 0);
                entries += 1;
            } else {
                assert_eq!(path.modules().count(), 1);
                nested += 1;
            }
        }
        assert_eq!((entries, nested), (1, 1));
    }

    #[test]
    fn representation_alias_requires_validated_equal_nominals() {
        fn setup(source: &str) -> (frontend::PlanCtx, ir::PlanBudget, ir::DeclId, Vec<ir::Ty>) {
            let program = frontend::parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let find = |name: &str| {
                (0..context.callable_count())
                    .map(|index| context.callable_decl(index).unwrap())
                    .find(|declaration| named(&context, *declaration, name))
                    .unwrap()
            };
            let generic = find("identity");
            let params = context
                .signature(find("types"))
                .unwrap()
                .signature
                .params
                .to_vec();
            (context, budget, generic, params)
        }
        let source = "struct A;struct B;fn identity<T>(x:T)->T{x}fn types(a:A,b:B){}fn main(){}";
        let (context, budget, generic, params) = setup(source);
        let target = |ty: &ir::Ty| ir::CallTarget {
            declaration: generic,
            type_args: Arc::from([ty.clone()]),
        };
        assert!(
            admit_representation_alias(&target(&params[0]), &target(&params[0]), &context, &budget)
                .unwrap()
        );
        assert!(
            !admit_representation_alias(
                &target(&params[0]),
                &target(&params[1]),
                &context,
                &budget
            )
            .unwrap()
        );
        // A nominal forged from another program's interner is never admitted,
        // even when its declaration index and arity coincide.
        let (_other, _, _, foreign) =
            setup("struct B;struct A;fn identity<T>(x:T)->T{x}fn types(a:A,b:B){}fn main(){}");
        assert!(!matches!(
            admit_representation_alias(
                &target(&params[0]),
                &target(&foreign[1]),
                &context,
                &budget
            ),
            Ok(true)
        ));
    }

    #[test]
    fn persistent_identity_rejects_foreign_declaration_before_encoding() {
        let source = "fn item()->i32{1}fn main(){}";
        let first_program = frontend::parse(source).unwrap();
        let second_program = frontend::parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let first = frontend::PlanCtx::from_program(&first_program, &budget).unwrap();
        let second = frontend::PlanCtx::from_program(&second_program, &budget).unwrap();
        let target = ir::CallTarget {
            declaration: first.callable_decl(0).unwrap(),
            type_args: Arc::from([]),
        };
        assert!(identity(&target, &first, &budget).is_ok());
        assert!(identity(&target, &second, &budget).is_err());
        assert!(second.is_entry_decl(target.declaration).is_err());
    }

    #[test]
    fn trait_implementation_keys_include_every_actual_and_ignore_discovery_order() {
        fn keys(source: &str) -> Vec<Vec<u8>> {
            let program = frontend::parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let mut keys = Vec::new();
            for index in 0..context.callable_count() {
                let declaration = context.callable_decl(index).unwrap();
                if context
                    .semantic_path(declaration)
                    .unwrap()
                    .associated
                    .as_ref()
                    .is_some_and(|owner| owner.trait_decl.is_some())
                {
                    keys.push(
                        identity(
                            &ir::CallTarget {
                                declaration,
                                type_args: Arc::from([]),
                            },
                            &context,
                            &budget,
                        )
                        .unwrap()
                        .canonical
                        .to_vec(),
                    );
                }
            }
            keys.sort();
            assert_eq!(keys.len(), 2);
            assert_ne!(keys[0], keys[1]);
            keys
        }
        assert_eq!(
            keys(
                "struct S;trait Get<T>{fn get(self)->T;}impl Get<i32> for S{fn get(self)->i32{7}}impl Get<bool> for S{fn get(self)->bool{true}}fn main(){}"
            ),
            keys(
                "fn unrelated(){}trait Get<T>{fn get(self)->T;}struct S;impl Get<bool> for S{fn get(self)->bool{true}}impl Get<i32> for S{fn get(self)->i32{7}}fn main(){}"
            ),
        );
        assert_eq!(
            keys(
                "struct S;trait Pair<A,B>{fn first(self,a:A,b:B)->A;}impl Pair<i32,bool> for S{fn first(self,a:i32,b:bool)->i32{a}}impl Pair<bool,i32> for S{fn first(self,a:bool,b:i32)->bool{a}}fn main(){}"
            ),
            keys(
                "trait Pair<A,B>{fn first(self,a:A,b:B)->A;}struct S;impl Pair<bool,i32> for S{fn first(self,a:bool,b:i32)->bool{a}}impl Pair<i32,bool> for S{fn first(self,a:i32,b:bool)->i32{a}}fn unrelated(){}fn main(){}"
            ),
        );
    }

    #[test]
    fn trait_default_keys_preserve_trait_owner_and_ignore_discovery_order() {
        fn keys(source: &str) -> Vec<Vec<u8>> {
            let program = frontend::parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let mut keys = Vec::new();
            for index in 0..context.callable_count() {
                let declaration = context.callable_decl(index).unwrap();
                if context.declaration(declaration).unwrap().kind
                    != ir::DeclKind::AssociatedFunction
                    || !matches!(
                        context.callable_provider(declaration).unwrap(),
                        frontend::CallableProvider::Source { .. }
                    )
                {
                    continue;
                }
                keys.push(
                    identity(
                        &ir::CallTarget {
                            declaration,
                            type_args: Arc::from([ir::Ty::Int(ir::IntTy::I32)]),
                        },
                        &context,
                        &budget,
                    )
                    .unwrap()
                    .canonical
                    .to_vec(),
                );
            }
            keys.sort();
            assert_eq!(keys.len(), 2);
            assert_ne!(keys[0], keys[1]);
            keys
        }
        let first = keys(
            "trait A:Sized{fn item(self)->i32{1}}trait B:Sized{fn item(self)->i32{2}}fn main(){}",
        );
        let reordered = keys(
            "fn unrelated(){}trait B:Sized{fn item(self)->i32{2}}trait A:Sized{fn item(self)->i32{1}}fn main(){}",
        );
        assert_eq!(first, reordered);
    }

    #[test]
    fn associated_keys_are_alpha_normalized_and_specialized_owners_are_distinct() {
        fn context(source: &str) -> (frontend::PlanCtx, ir::PlanBudget) {
            let program = frontend::parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            (context, budget)
        }
        fn methods(context: &frontend::PlanCtx, name: &str) -> Vec<ir::DeclId> {
            (0..context.callable_count())
                .map(|index| context.callable_decl(index).unwrap())
                .filter(|declaration| {
                    if !matches!(
                        context.callable_provider(*declaration).unwrap(),
                        frontend::CallableProvider::Source { .. }
                    ) {
                        return false;
                    }
                    let path = context.semantic_path(*declaration).unwrap();
                    path.associated.is_some() && named(context, *declaration, name)
                })
                .collect()
        }
        let (first, first_budget) =
            context("struct W<T>{x:T}impl<T> W<T>{fn pick<U>(self,u:U)->U{u}}fn main(){}");
        let (second, second_budget) = context(
            "fn unrelated(){}fn main(){}struct W<A>{x:A}impl<A> W<A>{fn pick<B>(self,u:B)->B{u}}",
        );
        let first_decl = methods(&first, "pick")[0];
        let second_decl = methods(&second, "pick")[0];
        assert_ne!(first_decl, second_decl);
        let target = |declaration| ir::CallTarget {
            declaration,
            type_args: Arc::from([ir::Ty::Int(ir::IntTy::I32), ir::Ty::Int(ir::IntTy::U8)]),
        };
        let first_key = identity(&target(first_decl), &first, &first_budget).unwrap();
        let second_key = identity(&target(second_decl), &second, &second_budget).unwrap();
        assert_eq!(first_key.canonical, second_key.canonical);
        assert_eq!(first_key.symbol, second_key.symbol);

        let (specialized, budget) = context(
            "struct W<T>{x:T}impl W<u8>{fn score(self)->i32{1}}impl W<u16>{fn score(self)->i32{2}}fn main(){}",
        );
        let owners = methods(&specialized, "score");
        assert_eq!(owners.len(), 2);
        let empty_target = |declaration| ir::CallTarget {
            declaration,
            type_args: Arc::from([]),
        };
        let small = identity(&empty_target(owners[0]), &specialized, &budget).unwrap();
        let wide = identity(&empty_target(owners[1]), &specialized, &budget).unwrap();
        assert_ne!(small.canonical, wide.canonical);
        assert_ne!(small.symbol, wide.symbol);
    }

    #[test]
    fn trait_owner_keys_distinguish_inherent_items_and_ignore_impl_discovery_order() {
        fn keys(source: &str) -> Vec<Vec<u8>> {
            let program = frontend::parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let mut result = Vec::new();
            for index in 0..context.callable_count() {
                let declaration = context.callable_decl(index).unwrap();
                if !matches!(
                    context.callable_provider(declaration).unwrap(),
                    frontend::CallableProvider::Source { .. }
                ) {
                    continue;
                }
                let path = context.semantic_path(declaration).unwrap();
                if named(&context, declaration, "item") && path.associated.is_some() {
                    result.push(
                        identity(
                            &ir::CallTarget {
                                declaration,
                                type_args: Arc::from([ir::Ty::Int(ir::IntTy::I32)]),
                            },
                            &context,
                            &budget,
                        )
                        .unwrap()
                        .canonical
                        .to_vec(),
                    );
                }
            }
            result.sort();
            assert_eq!(result.len(), 3);
            assert!(result.windows(2).all(|pair| pair[0] != pair[1]));
            result
        }
        let first = keys(
            "struct W<T>{x:T}trait A{fn item(self)->i32;}trait B{fn item(self)->i32;}impl<T> W<T>{fn item(self)->i32{1}}impl<T> A for W<T>{fn item(self)->i32{2}}impl<T> B for W<T>{fn item(self)->i32{3}}fn main(){}",
        );
        let reordered = keys(
            "trait B{fn item(self)->i32;}fn unrelated(){}struct W<U>{x:U}impl<X> B for W<X>{fn item(self)->i32{3}}trait A{fn item(self)->i32;}impl<Y> A for W<Y>{fn item(self)->i32{2}}impl<Z> W<Z>{fn item(self)->i32{1}}fn main(){}",
        );
        assert_eq!(first, reordered);
    }

    #[test]
    fn borrowed_receiver_keys_ignore_region_names_and_preserve_borrow_kind() {
        fn key(source: &str) -> Vec<u8> {
            let program = frontend::parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = (0..context.callable_count())
                .map(|index| context.callable_decl(index).unwrap())
                .filter(|declaration| {
                    matches!(
                        context.callable_provider(*declaration).unwrap(),
                        frontend::CallableProvider::Source { .. }
                    )
                })
                .find(|declaration| named(&context, *declaration, "value"))
                .unwrap();
            identity(
                &ir::CallTarget {
                    declaration,
                    type_args: Arc::from([]),
                },
                &context,
                &budget,
            )
            .unwrap()
            .canonical
            .to_vec()
        }
        let shared =
            key("struct S{x:i32}impl S{fn value<'a>(&'a self)->&'a i32{&self.x}}fn main(){}");
        let renamed = key(
            "fn unused(){}struct S{x:i32}impl S{fn value<'b>(&'b self)->&'b i32{&self.x}}fn main(){}",
        );
        let mutable = key(
            "struct S{x:i32}impl S{fn value<'a>(&'a mut self)->&'a mut i32{&mut self.x}}fn main(){}",
        );
        assert_eq!(shared, renamed);
        assert_ne!(shared, mutable);
    }

    #[test]
    fn generic_carrier_keys_preserve_reference_kind_topology_and_resource_bounds() {
        fn context(source: &str) -> (frontend::PlanCtx, ir::PlanBudget, ir::DeclId) {
            let program = frontend::parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = (0..context.callable_count())
                .map(|index| context.callable_decl(index).unwrap())
                .find(|declaration| {
                    matches!(
                        context.callable_provider(*declaration).unwrap(),
                        frontend::CallableProvider::Source { .. }
                    ) && named(&context, *declaration, "identity")
                })
                .unwrap();
            (context, budget, declaration)
        }
        let (first, budget, declaration) = context("fn identity<T>(x:T)->T{x}fn main(){}");
        let (reordered, other_budget, other_declaration) =
            context("fn unrelated(){}fn main(){}fn identity<U>(x:U)->U{x}");
        let reference = |kind, referent| ir::Ty::Ref {
            kind,
            referent: Arc::new(referent),
        };
        let scalar = ir::Ty::Int(ir::IntTy::I32);
        let shared = reference(ir::BorrowKind::Shared, scalar.clone());
        let mutable = reference(ir::BorrowKind::Mut, scalar.clone());
        let nested = reference(ir::BorrowKind::Shared, shared.clone());
        let target = |declaration, ty| ir::CallTarget {
            declaration,
            type_args: Arc::from([ty]),
        };
        let shared_key = identity(&target(declaration, shared.clone()), &first, &budget).unwrap();
        let renamed_key = identity(
            &target(other_declaration, shared.clone()),
            &reordered,
            &other_budget,
        )
        .unwrap();
        assert_eq!(shared_key.canonical, renamed_key.canonical);
        assert_eq!(shared_key.symbol, renamed_key.symbol);
        for different in [scalar.clone(), mutable, nested] {
            let key = identity(&target(declaration, different), &first, &budget).unwrap();
            assert_ne!(shared_key.canonical, key.canonical);
            assert_ne!(shared_key.symbol, key.symbol);
        }
        let wide_keys = [
            ir::IntTy::I64,
            ir::IntTy::U64,
            ir::IntTy::I128,
            ir::IntTy::U128,
        ]
        .map(|integer| {
            let ty = ir::Ty::Int(integer);
            let first_key = identity(&target(declaration, ty.clone()), &first, &budget).unwrap();
            let reordered_key =
                identity(&target(other_declaration, ty), &reordered, &other_budget).unwrap();
            assert_eq!(first_key.canonical, reordered_key.canonical);
            assert_eq!(first_key.symbol, reordered_key.symbol);
            first_key.symbol
        });
        assert_eq!(
            wide_keys
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            wide_keys.len()
        );
        let char_key = identity(&target(declaration, ir::Ty::Char), &first, &budget).unwrap();
        let renamed_char = identity(
            &target(other_declaration, ir::Ty::Char),
            &reordered,
            &other_budget,
        )
        .unwrap();
        assert_eq!(char_key.canonical, renamed_char.canonical);
        for different in [
            ir::Ty::Int(ir::IntTy::U32),
            ir::Ty::Bool,
            ir::Ty::Float(ir::FloatTy::F32),
        ] {
            let key = identity(&target(declaration, different), &first, &budget).unwrap();
            assert_ne!(char_key.canonical, key.canonical);
            assert_ne!(char_key.symbol, key.symbol);
        }
        let mut excessive = scalar;
        for _ in 0..=ir::PlanLimits::default().depth {
            excessive = reference(ir::BorrowKind::Shared, excessive);
        }
        assert!(identity(&target(declaration, excessive), &first, &budget).is_err());
        let exhausted = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..ir::PlanLimits::default()
        });
        assert!(identity(&target(declaration, shared), &first, &exhausted).is_err());
    }

    #[test]
    fn normal_drop_demand_is_sealed_even_when_destructor_is_already_a_source_root() {
        let text =
            "fn f(){let d=D{v:1};}struct D{v:i32}impl Drop for D{fn drop(&mut self){}}fn main(){}";
        let program = frontend::parse(text).unwrap();
        let budget = ir::PlanBudget::default();
        let mut context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let mut sources = Vec::new();
        let mut publication = None;
        for index in 0..context.callable_count() {
            let declaration = context.callable_decl(index).unwrap();
            match context.publish_callable_body(declaration, &budget).unwrap() {
                frontend::PublishedCallableBody::Source(source) => {
                    if declaration == context.function_decl(0).unwrap() {
                        publication = Some(source.clone());
                    }
                    sources.push(source.source().clone());
                }
                frontend::PublishedCallableBody::Recipe(source) => sources.push(source),
            }
        }
        let publication = publication.unwrap();
        let function = super::super::cfg::lower_source_closed(&publication, &budget).unwrap();
        let mut plan = build(&mut context, &sources, &budget, None).unwrap();
        assert!(!plan.cleanup_targets.is_empty());
        plan.validate_cleanup_dependencies(&function).unwrap();
        let demanded = plan.cleanup_targets.clone();
        plan.cleanup_targets.clear();
        assert!(
            plan.symbol(&demanded[0]).is_ok(),
            "source-root lookup should still succeed"
        );
        assert!(
            plan.validate_cleanup_dependencies(&function)
                .unwrap_err()
                .contains("implicit instance demand")
        );
        plan.cleanup_targets = demanded.clone();
        plan.lookup.remove(&demanded[0]);
        assert!(plan.validate_cleanup_dependencies(&function).is_err());
    }

    #[test]
    fn generic_normal_drop_is_relowered_and_demands_its_destructor() {
        let text = "fn consume<T>(value:T){}struct D{v:i32}impl Drop for D{fn drop(&mut self){}}fn main(){consume::<D>(D{v:1});}";
        let program = frontend::parse(text).unwrap();
        let budget = ir::PlanBudget::default();
        let mut context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = context.function_decl(0).unwrap();
        let mut sources = Vec::new();
        let mut publication = None;
        for index in 0..context.callable_count() {
            let current = context.callable_decl(index).unwrap();
            match context.publish_callable_body(current, &budget).unwrap() {
                frontend::PublishedCallableBody::Source(source) => {
                    if current == declaration {
                        publication = Some(source.clone());
                    }
                    sources.push(source.source().clone());
                }
                frontend::PublishedCallableBody::Recipe(source) => sources.push(source),
            }
        }
        let publication = publication.unwrap();
        let template = super::super::cfg::lower_source_template(&publication, &budget).unwrap();
        let verified = super::super::ownership::verify_template(template).unwrap();
        assert!(!verified.function().blocks.iter().any(|block| matches!(
            block.terminator.value,
            super::super::cfg::Terminator::Drop { .. }
        )));
        let mut plan = build(&mut context, &sources, &budget, None).unwrap();
        let target = plan
            .entries
            .iter()
            .find(|entry| entry.target.declaration == declaration)
            .unwrap()
            .target
            .clone();
        let facts = context.instance_facts(&target).unwrap();
        let checked =
            ir::instantiate_checked(&verified, &target.type_args, &facts, &mut context, &budget)
                .unwrap();
        let function = super::super::cfg::lower_with_facts(&checked, &facts, &budget).unwrap();
        plan.validate_cleanup_dependencies(&function).unwrap();
        let site = function
            .blocks
            .iter()
            .position(|block| {
                matches!(
                    block.terminator.value,
                    super::super::cfg::Terminator::Drop { .. }
                )
            })
            .unwrap();
        let mut missing = function.clone();
        let super::super::cfg::Terminator::Drop { next, .. } =
            missing.blocks[site].terminator.value
        else {
            unreachable!()
        };
        missing.blocks[site].terminator.value = super::super::cfg::Terminator::Goto(next);
        assert!(super::super::cfg::close(Arc::new(missing), facts.clone(), &budget).is_err());
        let mut stale = function.clone();
        stale.lowering_proof = verified.function().lowering_proof.clone();
        assert!(super::super::cfg::close(Arc::new(stale), facts.clone(), &budget).is_err());
        super::super::cfg::close(Arc::new(function.clone()), facts, &budget).unwrap();
        plan.cleanup_targets.clear();
        assert!(
            plan.validate_cleanup_dependencies(&function)
                .unwrap_err()
                .contains("implicit instance demand")
        );
    }

    #[test]
    fn generic_owner_cleanup_targets_keep_concrete_type_arguments() {
        let text = "struct D<T>{pointer:*mut T}impl<T> Drop for D<T>{fn drop(&mut self){}}fn main(){let a=D::<i32>{pointer:0usize as *mut i32};let b=D::<u8>{pointer:0usize as *mut u8};}";
        let program = frontend::parse(text).unwrap();
        let budget = ir::PlanBudget::default();
        let mut context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let mut sources = Vec::new();
        for index in 0..context.callable_count() {
            let current = context.callable_decl(index).unwrap();
            match context.publish_callable_body(current, &budget).unwrap() {
                frontend::PublishedCallableBody::Source(source) => {
                    sources.push(source.source().clone())
                }
                frontend::PublishedCallableBody::Recipe(source) => sources.push(source),
            }
        }
        let plan = build(&mut context, &sources, &budget, None).unwrap();
        assert_eq!(plan.cleanup_targets.len(), 2);
        let first = &plan.cleanup_targets[0];
        let second = &plan.cleanup_targets[1];
        assert_eq!(first.declaration, second.declaration);
        assert_eq!(first.type_args.len(), 1);
        assert_eq!(second.type_args.len(), 1);
        assert_ne!(first.type_args, second.type_args);
        assert_ne!(plan.symbol(first).unwrap(), plan.symbol(second).unwrap());
    }

    #[test]
    fn relocation_symbol_copies_share_the_plan_byte_limit() {
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 12 + 4 * std::mem::size_of::<(usize, String)>(),
            ..Default::default()
        });
        let target = ir::CallTarget {
            declaration: ir::DeclId {
                snapshot: ir::SnapshotId::standalone().unwrap(),
                index: 0,
            },
            type_args: Arc::from([]),
        };
        let mut lookup = HashMap::new();
        lookup.insert(target.clone(), 0);
        let plan = Plan {
            entries: vec![Instance {
                target: target.clone(),
                source_index: 0,
                identity: InstanceIdentity {
                    symbol: "$jr$abcd".into(),
                    canonical: Arc::from([]),
                    _reservations: vec![],
                },
            }],
            cleanup_targets: vec![],
            lookup,
            _reservations: vec![],
            budget: budget.clone(),
        };
        assert_eq!(plan.symbol(&target).unwrap(), "$jr$abcd");
        assert!(plan.symbol(&target).unwrap_err().contains("limit"));
        assert!(budget.reserve_bytes(5).is_err());
    }
}
