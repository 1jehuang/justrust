//! Snapshot-local instance planning with canonical, collision-free native symbols.
//! Local declaration/application IDs are lookup handles, never persistent keys.
use super::{frontend, ir};
use std::sync::Arc;

const SEMANTICS: &[u8] = b"jr-native-generics-v1/linux-x86_64/checked/private-leaf8";

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
        let path = self.context.semantic_path(declaration)?;
        self.append(&[match path.kind {
            ir::DeclKind::Function => 0,
            ir::DeclKind::Struct => 1,
            ir::DeclKind::Enum => 2,
        }])?;
        self.count(path.module.len())?;
        for component in path.module.iter() {
            self.text(component)?;
        }
        self.text(&path.item)
    }

    fn ty(&mut self, ty: &ir::Ty, depth: usize) -> Result<(), String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        match ty {
            ir::Ty::Int(integer) => {
                self.append(b"i")?;
                self.text(integer.name())
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
            ir::Ty::Ref { .. } => Err("unsupported: reference-containing generic argument".into()),
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
    let canonical_reservation = budget.reserve_bytes(encoder.bytes.len())?;
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
    encoder.reservations.push(symbol_reservation);
    encoder.reservations.push(canonical_reservation);
    Ok(InstanceIdentity {
        symbol,
        canonical: Arc::from(encoder.bytes),
        _reservations: encoder.reservations,
    })
}

use std::collections::HashMap;

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
        let index = self
            .lookup
            .get(target)
            .ok_or("call target is not in the sealed generic instance plan")?;
        let symbol = &self.entries[*index].identity.symbol;
        self.budget.charge_work(symbol.len())?;
        self.budget.retain_bytes(symbol.len())?;
        let mut copy = String::new();
        copy.try_reserve_exact(symbol.len())
            .map_err(|_| "generic relocation symbol allocation limit exceeded")?;
        copy.push_str(symbol);
        Ok(copy)
    }
}

pub(super) fn build(
    program: &frontend::Program,
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
        program
            .functions
            .len()
            .checked_mul(4 * (std::mem::size_of::<ir::DeclId>() + std::mem::size_of::<usize>()))
            .ok_or("generic declaration lookup byte limit exceeded")?,
    )?;
    declarations
        .try_reserve(program.functions.len())
        .map_err(|_| "generic declaration lookup allocation limit exceeded")?;
    for index in 0..program.functions.len() {
        budget.charge_work(1)?;
        let declaration = context.function_decl(index)?;
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
            fn charge_type(
                ty: &ir::Ty,
                depth: usize,
                budget: &ir::PlanBudget,
            ) -> Result<(), String> {
                budget.check_depth(depth)?;
                budget.charge_work(1)?;
                match ty {
                    ir::Ty::Tuple(fields) => {
                        for field in fields.iter() {
                            charge_type(field, depth + 1, budget)?;
                        }
                    }
                    ir::Ty::Array { element, .. } => charge_type(element, depth + 1, budget)?,
                    // Nominal Eq/Hash use canonical application identity, not fields.
                    ir::Ty::Struct(_)
                    | ir::Ty::Enum(_)
                    | ir::Ty::Int(_)
                    | ir::Ty::Bool
                    | ir::Ty::Unit
                    | ir::Ty::Never => {}
                    ir::Ty::Param(_) | ir::Ty::Ref { .. } => {
                        return Err(
                            "generic instance has non-closed or reference-containing argument"
                                .into(),
                        );
                    }
                }
                Ok(())
            }
            budget.charge_work(1)?;
            for argument in target.type_args.iter() {
                charge_type(argument, 0, budget)?;
            }
            // Exact recursive instances are already finite, regardless of call depth.
            if self.seen.contains_key(&target) {
                return Ok(());
            }
            budget.check_depth(depth)?;
            let source_index = *self
                .declarations
                .get(&target.declaration)
                .ok_or("generic call does not resolve to a function declaration")?;
            ir::SubstitutionCx::instantiate_signature(
                context,
                &target,
                ir::FactsCtx::Concrete,
                budget,
            )?;
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
            if path.module.is_empty() && path.item == "main" && target.type_args.is_empty() {
                identity.symbol.clear();
                identity.symbol.push_str("main");
            }
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
    for (index, function) in program.functions.iter().enumerate() {
        if function.type_params.is_empty() {
            queue.add(
                ir::CallTarget {
                    declaration: context.function_decl(index)?,
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
        let binder = schema.param_env.binder();
        let source = sources
            .get(source_index)
            .ok_or("missing universally checked declaration body")?;
        for expression in &source.exprs {
            budget.charge_work(1)?;
            if let ir::ExprKind::Call { target: callee, .. } = &expression.kind {
                let count = callee.type_args.len();
                let scratch = budget.reserve_bytes(
                    count
                        .checked_mul(2 * std::mem::size_of::<ir::Ty>())
                        .ok_or("generic call argument byte limit exceeded")?,
                )?;
                let mut arguments = Vec::new();
                arguments
                    .try_reserve_exact(count)
                    .map_err(|_| "generic call argument allocation limit exceeded")?;
                for argument in callee.type_args.iter() {
                    arguments.push(ir::substitute_ty(
                        argument,
                        binder,
                        &target.type_args,
                        ir::FactsCtx::Concrete,
                        context,
                        budget,
                    )?);
                }
                let next = ir::CallTarget {
                    declaration: callee.declaration,
                    type_args: Arc::from(arguments),
                };
                // Retain the argument storage charge until the sealed plan dies.
                queue
                    .reservations
                    .try_reserve_exact(1)
                    .map_err(|_| "generic plan reservation allocation limit exceeded")?;
                queue.reservations.push(scratch);
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
        budget.charge_work(1)?;
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
    fn relocation_symbol_copies_share_the_plan_byte_limit() {
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 12,
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
