//! Snapshot-local instance planning with canonical, collision-free native symbols.
//! Local declaration/application IDs are lookup handles, never persistent keys.
use super::{frontend, ir, traits};
use std::sync::Arc;

const SEMANTICS: &[u8] =
    b"jr-native-generic-carriers-v1/linux-x86_64/checked/carrier-regions-storage-effects";

pub(super) struct InstanceIdentity {
    pub symbol: String,
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
        let path = self.context.semantic_path(declaration)?;
        self.append(&[match path.kind {
            ir::DeclKind::Function => 0,
            ir::DeclKind::Struct => 1,
            ir::DeclKind::Enum => 2,
            ir::DeclKind::Impl => 3,
            ir::DeclKind::AssociatedFunction => 4,
            ir::DeclKind::Trait => 5,
            ir::DeclKind::TraitItem => 6,
            ir::DeclKind::TraitImpl => 7,
        }])?;
        self.count(path.module.len())?;
        for component in path.module.iter() {
            self.text(component)?;
        }
        self.text(&path.item)?;
        if let Some(owner) = &path.associated {
            self.append(b"A")?;
            let impl_path = self.context.semantic_path(owner.impl_decl)?;
            self.count(impl_path.module.len())?;
            for component in impl_path.module.iter() {
                self.text(component)?;
            }
            self.declaration(owner.nominal)?;
            let env = self.context.param_env(owner.impl_decl)?;
            self.count(env.arity())?;
            self.pattern(&owner.self_pattern, &env, 0)?;
            match (owner.trait_decl, owner.trait_item) {
                (None, None) => self.append(b"I")?,
                (Some(trait_decl), Some(trait_item)) => {
                    self.append(b"T")?;
                    self.declaration(trait_decl)?;
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
        } else {
            self.append(b"F")
        }
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
            ir::Ty::Never => Err("unsupported associated instance self pattern".into()),
        }
    }

    fn ty(&mut self, ty: &ir::Ty, depth: usize) -> Result<(), String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        match ty {
            ir::Ty::Int(integer) => {
                self.append(b"i")?;
                self.text(integer.name())
            }
            ir::Ty::Float(float) => {
                self.append(b"f")?;
                self.text(float.name())
            }
            ir::Ty::Bool => self.append(b"b"),
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
            ir::Ty::Param(_) => Err("generic instance key contains unresolved parameter".into()),
            ir::Ty::Ref { kind, referent } => {
                self.append(match kind {
                    ir::BorrowKind::Shared => b"rs",
                    ir::BorrowKind::Mut => b"rm",
                })?;
                self.ty(referent, depth + 1)
            }
        }
    }
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

use std::collections::HashMap;

fn charge_target_lookup(target: &ir::CallTarget, budget: &ir::PlanBudget) -> Result<(), String> {
    fn walk(value: &ir::Ty, depth: usize, budget: &ir::PlanBudget) -> Result<(), String> {
        budget.check_depth(depth)?;
        // Cover the bounded group of Hash/Eq operations following this walk.
        budget.charge_work(5)?;
        match value {
            ir::Ty::Tuple(fields) => {
                for field in fields.iter() {
                    walk(field, depth + 1, budget)?;
                }
            }
            ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                walk(element, depth + 1, budget)?
            }
            ir::Ty::Struct(_)
            | ir::Ty::Enum(_)
            | ir::Ty::Int(_)
            | ir::Ty::Float(_)
            | ir::Ty::Bool
            | ir::Ty::Unit
            | ir::Ty::Never => {}
            ir::Ty::Ref { referent, .. } => walk(referent, depth + 1, budget)?,
            ir::Ty::Param(_) => {
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

pub(super) struct Instance {
    pub target: ir::CallTarget,
    pub source_index: usize,
    pub identity: InstanceIdentity,
}

pub(super) struct Plan {
    pub entries: Vec<Instance>,
    lookup: HashMap<ir::CallTarget, usize>,
    _reservations: Vec<ir::PlanReservation>,
    budget: ir::PlanBudget,
}

impl Plan {
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

pub(super) fn build(
    context: &mut frontend::PlanCtx,
    sources: &[Arc<ir::CheckedFunction>],
    budget: &ir::PlanBudget,
) -> Result<Plan, String> {
    let mut declarations = HashMap::new();
    let mut entries = Vec::new();
    let mut seen = HashMap::new();
    let mut depths = Vec::new();
    let mut reservations = Vec::new();
    let mut canonical_keys = HashMap::new();
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
    }
    impl Queue<'_> {
        fn add(
            &mut self,
            target: ir::CallTarget,
            depth: usize,
            context: &mut frontend::PlanCtx,
            budget: &ir::PlanBudget,
        ) -> Result<(), String> {
            charge_target_lookup(&target, budget)?;
            // A cached target or canonical nominal app is not admission authority.
            // Validate the exact instance even for recursive/cache-hit demands.
            context.instance_facts(&target)?;
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
            let path = context.semantic_path(target.declaration)?;
            if path.kind == ir::DeclKind::Function
                && path.associated.is_none()
                && path.module.is_empty()
                && path.item == "main"
                && target.type_args.is_empty()
            {
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
            {
                return Err("generic canonical instance identity collision".into());
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
    let mut queue = Queue {
        declarations: &declarations,
        entries: &mut entries,
        seen: &mut seen,
        depths: &mut depths,
        canonical_keys: &mut canonical_keys,
        reservations: &mut reservations,
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
        if source && !context.callable_env(declaration)?.is_generic() {
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
        let schema = context.function_schema(target.declaration)?;
        let env = schema.env;
        let closed_facts = context.instance_facts(&target)?;
        let source = sources
            .get(source_index)
            .ok_or("missing universally checked declaration body")?;
        for expression in &source.exprs {
            budget.charge_work(1)?;
            if let ir::ExprKind::Call { target: callee, .. } = &expression.kind {
                let resolved = ir::substitute_callee(
                    callee,
                    &env,
                    &target.type_args,
                    &closed_facts,
                    context,
                    budget,
                )?;
                let ir::Callee::Direct(next) = resolved else {
                    return Err("unresolved trait callee in concrete instance frontier".into());
                };
                queue.add(next, depth + 1, context, budget)?;
            }
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
    reservations.push(lookup_reservation);
    Ok(Plan {
        entries,
        lookup,
        _reservations: reservations,
        budget: budget.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    path.associated.is_some() && path.item == name
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
                if path.item == "item" && path.associated.is_some() {
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
                .find(|declaration| context.semantic_path(*declaration).unwrap().item == "value")
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
                    ) && context.semantic_path(*declaration).unwrap().item == "identity"
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
    fn relocation_symbol_copies_share_the_plan_byte_limit() {
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 12 + 4 * std::mem::size_of::<(usize, String)>(),
            ..Default::default()
        });
        let target = ir::CallTarget {
            declaration: ir::DeclId {
                snapshot: ir::SnapshotId::allocate().unwrap(),
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
            lookup,
            _reservations: vec![],
            budget: budget.clone(),
        };
        assert_eq!(plan.symbol(&target).unwrap(), "$jr$abcd");
        assert!(plan.symbol(&target).unwrap_err().contains("limit"));
        assert!(budget.reserve_bytes(5).is_err());
    }
}
