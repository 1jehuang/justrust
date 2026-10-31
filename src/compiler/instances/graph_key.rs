//! Injective process-local graph coordinates for canonical callable instances.
//! These are not persistent symbols. Nominals encode declaration applications,
//! never allocation-order-dependent application IDs or diagnostic names.
use super::*;

const VERSION: u64 = 0x4a52_494e_5354_0001;
const INTS: [ir::IntTy; 12] = [
    ir::IntTy::I8,
    ir::IntTy::I16,
    ir::IntTy::I32,
    ir::IntTy::I64,
    ir::IntTy::I128,
    ir::IntTy::Isize,
    ir::IntTy::U8,
    ir::IntTy::U16,
    ir::IntTy::U32,
    ir::IntTy::U64,
    ir::IntTy::U128,
    ir::IntTy::Usize,
];

pub(in crate::compiler) fn encode(
    target: &ir::CallTarget,
    budget: &ir::PlanBudget,
) -> Result<Vec<u64>, String> {
    let mut words = vec![VERSION];
    declaration(&mut words, target.declaration);
    types(&mut words, &target.type_args, budget, 0)?;
    budget.charge_work(words.len())?;
    budget.retain_bytes(
        words
            .len()
            .checked_mul(16)
            .ok_or("instance key size overflow")?,
    )?;
    Ok(words)
}

fn declaration(words: &mut Vec<u64>, id: ir::DeclId) {
    words.extend([id.snapshot.graph_word(), id.index]);
}
fn types(
    words: &mut Vec<u64>,
    values: &[ir::Ty],
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.charge_work(values.len())?;
    words.push(values.len() as u64);
    for value in values {
        ty(words, value, budget, depth + 1)?;
    }
    Ok(())
}
fn ty(
    words: &mut Vec<u64>,
    value: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let start = words.len();
    words.extend([0, 0]);
    let tag = match value {
        ir::Ty::Int(value) => {
            words.push(
                INTS.iter()
                    .position(|candidate| candidate == value)
                    .ok_or("unknown integer key type")? as u64,
            );
            1
        }
        ir::Ty::Float(value) => {
            words.push(match value {
                ir::FloatTy::F32 => 0,
                ir::FloatTy::F64 => 1,
            });
            2
        }
        ir::Ty::Bool => 3,
        ir::Ty::Unit => 4,
        ir::Ty::Never => 5,
        ir::Ty::Char => 6,
        ir::Ty::Str => 7,
        ir::Ty::Ref { kind, referent } => {
            words.push(borrow(*kind));
            ty(words, referent, budget, depth + 1)?;
            8
        }
        ir::Ty::RawPtr { kind, pointee } => {
            words.push(borrow(*kind));
            ty(words, pointee, budget, depth + 1)?;
            9
        }
        ir::Ty::Tuple(fields) => {
            if fields.is_empty() {
                return Err("noncanonical empty tuple in instance key".into());
            }
            types(words, fields, budget, depth + 1)?;
            10
        }
        ir::Ty::Array { element, len } => {
            words.push(*len as u64);
            ty(words, element, budget, depth + 1)?;
            11
        }
        ir::Ty::Slice { element } => {
            ty(words, element, budget, depth + 1)?;
            12
        }
        ir::Ty::Struct(value) => {
            declaration(words, value.def().source_decl);
            types(words, &value.def().type_args, budget, depth + 1)?;
            13
        }
        ir::Ty::Enum(value) => {
            declaration(words, value.def().source_decl);
            types(words, &value.def().type_args, budget, depth + 1)?;
            14
        }
        ir::Ty::Dyn(value) => {
            if !value.args.is_empty() || !value.associated_types.is_empty() {
                return Err(
                    "unsupported: rich dyn instance graph key requires complete principal metadata"
                        .into(),
                );
            }
            declaration(words, value.principal);
            15
        }
        ir::Ty::Opaque(value) => {
            declaration(words, value.owner());
            words.push(u64::from(value.site()));
            types(words, value.owner_args(), budget, depth + 1)?;
            words.push(value.captured_regions().len() as u64);
            for region in value.captured_regions() {
                declaration(words, region.declaration);
                words.push(u64::from(region.index));
            }
            words.push(value.bounds().len() as u64);
            for bound in value.bounds() {
                declaration(words, bound.declaration());
                words.push(bound.args().len() as u64);
                for arg in bound.args() {
                    match arg {
                        ir::OpaqueBoundArg::SelfType => words.push(0),
                        ir::OpaqueBoundArg::Type(value) => {
                            words.push(1);
                            ty(words, value, budget, depth + 1)?;
                        }
                    }
                }
                words.push(bound.equalities().len() as u64);
                for (item, value) in bound.equalities() {
                    declaration(words, *item);
                    ty(words, value, budget, depth + 1)?;
                }
            }
            16
        }
        ir::Ty::Closure(value) => {
            // The provider names the parent-minted template; the ordered owner
            // actuals are the only free coordinates. Decoding replays them
            // through the template, never a copied shape.
            declaration(words, value.provider());
            types(words, value.owner_args(), budget, depth + 1)?;
            17
        }
        ir::Ty::FnPointer(value) => {
            types(words, &value.inputs, budget, depth + 1)?;
            ty(words, &value.output, budget, depth + 1)?;
            19
        }
        ir::Ty::FnItem(value) => {
            declaration(words, value.declaration);
            types(words, &value.type_args, budget, depth + 1)?;
            18
        }
        ir::Ty::Param(_) | ir::Ty::AssociatedType(_) => {
            return Err("instance graph key requires closed actuals".into());
        }
    };
    words[start] = tag;
    words[start + 1] = (words.len() - start - 2) as u64;
    Ok(())
}
fn borrow(kind: ir::BorrowKind) -> u64 {
    match kind {
        ir::BorrowKind::Shared => 0,
        ir::BorrowKind::Mut => 1,
    }
}

#[cfg(test)]
pub(in crate::compiler) fn decode(
    words: &[u64],
    context: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<ir::CallTarget, String> {
    decode_with_closures(words, context, budget, &|_, _| {
        Err("unsupported: canonical closure graph key requires parent metadata replay".into())
    })
}

/// Parent-minted closure templates keyed by provider declaration, read from
/// each provider's `TEMPLATE_BODY` graph output.
pub(in crate::compiler) type ClosureTemplates =
    std::collections::HashMap<ir::DeclId, Arc<ir::ClosureTy>>;

/// Replays a tag-17 closure actual from its provider template and ordered owner
/// actuals through the closed-owner substitution kernel. A provider absent from
/// `templates` is recorded in `missing` so the graph caller can request it.
pub(in crate::compiler) fn replay_closure(
    context: &frontend::PlanCtx,
    templates: &ClosureTemplates,
    missing: &std::cell::RefCell<Vec<ir::DeclId>>,
    budget: &ir::PlanBudget,
    provider: ir::DeclId,
    owner_args: &[ir::Ty],
) -> Result<ir::Ty, String> {
    let Some(template) = templates.get(&provider) else {
        missing.borrow_mut().push(provider);
        return Err("closure graph key template not yet read".into());
    };
    if template.provider() != provider {
        return Err("closure graph key template changed its provider".into());
    }
    let owner = context.instance_facts(&ir::CallTarget {
        declaration: template.owner(),
        type_args: Arc::from(owner_args),
    })?;
    owner
        .instantiate_closure_owner(template, owner_args)
        .map(ir::Ty::Closure)
        .map_err(|e| e.into_legacy_diagnostic(budget))
}

/// Closure actual replay: `(provider, ordered owner actuals)` to the exact
/// substituted parent-minted closure type.
pub(in crate::compiler) type ClosureReplay<'a> =
    dyn Fn(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, String> + 'a;

pub(in crate::compiler) fn decode_with_closures(
    words: &[u64],
    context: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
    closures: &ClosureReplay<'_>,
) -> Result<ir::CallTarget, String> {
    budget.charge_work(words.len())?;
    let mut reader = Reader {
        words,
        at: 0,
        context,
        budget,
        closures,
    };
    if reader.word()? != VERSION {
        return Err("unsupported canonical instance graph key version".into());
    }
    let declaration = reader.declaration()?;
    let facts = context.callable_facts(declaration)?;
    let type_args = reader.types(ir::FactsCtx::from_validated(&facts), 0)?;
    if reader.at != words.len() {
        return Err("trailing instance graph key words".into());
    }
    let target = ir::CallTarget {
        declaration,
        type_args: type_args.into(),
    };
    // Metadata decoding grants no executable permission. Replay exact target
    // obligations through canonical instance facts before returning a target.
    context.instance_facts(&target)?;
    if encode(&target, budget)?.as_slice() != words {
        return Err("noncanonical instance graph key".into());
    }
    Ok(target)
}
pub(in crate::compiler) fn encode_opaque(
    opaque: &Arc<ir::OpaqueTy>,
    budget: &ir::PlanBudget,
) -> Result<Vec<u64>, String> {
    let mut words = vec![VERSION];
    declaration(&mut words, opaque.owner());
    ty(&mut words, &ir::Ty::Opaque(opaque.clone()), budget, 0)?;
    budget.charge_work(words.len())?;
    budget.retain_bytes(
        words
            .len()
            .checked_mul(16)
            .ok_or("opaque key size overflow")?,
    )?;
    Ok(words)
}

#[cfg(test)]
pub(in crate::compiler) fn decode_opaque(
    words: &[u64],
    context: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<Arc<ir::OpaqueTy>, String> {
    decode_opaque_with_closures(words, context, budget, &|_, _| {
        Err("unsupported: canonical closure graph key requires parent metadata replay".into())
    })
}

pub(in crate::compiler) fn decode_opaque_with_closures(
    words: &[u64],
    context: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
    closures: &ClosureReplay<'_>,
) -> Result<Arc<ir::OpaqueTy>, String> {
    budget.charge_work(words.len())?;
    let mut reader = Reader {
        words,
        at: 0,
        context,
        budget,
        closures,
    };
    if reader.word()? != VERSION {
        return Err("unsupported opaque graph key version".into());
    }
    let owner = reader.declaration()?;
    let facts = context.callable_facts(owner)?;
    let ir::Ty::Opaque(opaque) = reader.ty(ir::FactsCtx::from_validated(&facts), 0)? else {
        return Err("opaque graph key is not an opaque type".into());
    };
    if reader.at != words.len() || opaque.owner() != owner {
        return Err("opaque graph key changed its owner or extent".into());
    }
    context.instance_facts(&ir::CallTarget {
        declaration: owner,
        type_args: Arc::from(opaque.owner_args()),
    })?;
    if encode_opaque(&opaque, budget)?.as_slice() != words {
        return Err("noncanonical opaque graph key".into());
    }
    Ok(opaque)
}

struct Reader<'a> {
    words: &'a [u64],
    at: usize,
    context: &'a frontend::PlanCtx,
    budget: &'a ir::PlanBudget,
    closures: &'a ClosureReplay<'a>,
}
impl Reader<'_> {
    fn word(&mut self) -> Result<u64, String> {
        let value = *self
            .words
            .get(self.at)
            .ok_or("truncated instance graph key")?;
        self.at += 1;
        Ok(value)
    }
    fn count(&mut self) -> Result<usize, String> {
        let count =
            usize::try_from(self.word()?).map_err(|_| "instance graph key count overflow")?;
        if count > self.words.len().saturating_sub(self.at) {
            return Err("instance graph key count exceeds input".into());
        }
        Ok(count)
    }
    fn declaration(&mut self) -> Result<ir::DeclId, String> {
        let id = ir::DeclId {
            snapshot: ir::SnapshotId::from_graph_word(self.word()?),
            index: self.word()?,
        };
        self.context.declaration(id)?;
        Ok(id)
    }
    fn types(&mut self, facts: ir::FactsCtx<'_>, depth: usize) -> Result<Vec<ir::Ty>, String> {
        self.budget.check_depth(depth)?;
        let count = self.count()?;
        self.budget.charge_work(count)?;
        self.budget.retain_bytes(
            count
                .checked_mul(std::mem::size_of::<ir::Ty>() * 2)
                .ok_or("instance key allocation overflow")?,
        )?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(count)
            .map_err(|_| "instance key allocation failed")?;
        for _ in 0..count {
            output.push(self.ty(facts, depth + 1)?);
        }
        Ok(output)
    }
    fn kind(&mut self) -> Result<ir::BorrowKind, String> {
        match self.word()? {
            0 => Ok(ir::BorrowKind::Shared),
            1 => Ok(ir::BorrowKind::Mut),
            _ => Err("invalid instance key borrow kind".into()),
        }
    }
    fn ty(&mut self, facts: ir::FactsCtx<'_>, depth: usize) -> Result<ir::Ty, String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        let tag = self.word()?;
        let length = self.count()?;
        let end = self
            .at
            .checked_add(length)
            .ok_or("instance key frame overflow")?;
        let outer_words = self.words;
        self.words = &outer_words[..end];
        let result = (|| {
            if tag != 1 {
                return self.other_ty(tag, end, facts, depth);
            }
            let ty = *INTS
                .get(usize::try_from(self.word()?).map_err(|_| "integer key overflow")?)
                .ok_or("unknown instance integer tag")?;
            if self.at != end {
                return Err("instance key scalar frame length mismatch".into());
            }
            Ok(ir::Ty::Int(ty))
        })();
        self.words = outer_words;
        result
    }
    fn other_ty(
        &mut self,
        tag: u64,
        end: usize,
        facts: ir::FactsCtx<'_>,
        depth: usize,
    ) -> Result<ir::Ty, String> {
        let value = match tag {
            2 => ir::Ty::Float(match self.word()? {
                0 => ir::FloatTy::F32,
                1 => ir::FloatTy::F64,
                _ => return Err("invalid float key tag".into()),
            }),
            3 => ir::Ty::Bool,
            4 => ir::Ty::Unit,
            5 => ir::Ty::Never,
            6 => ir::Ty::Char,
            7 => ir::Ty::Str,
            8 => ir::Ty::Ref {
                kind: self.kind()?,
                referent: Arc::new(self.ty(facts, depth + 1)?),
            },
            9 => ir::Ty::RawPtr {
                kind: self.kind()?,
                pointee: Arc::new(self.ty(facts, depth + 1)?),
            },
            10 => {
                let fields = self.types(facts, depth + 1)?;
                if fields.is_empty() {
                    return Err("noncanonical empty tuple graph key".into());
                }
                ir::Ty::Tuple(fields.into())
            }
            11 => {
                let len = usize::try_from(self.word()?).map_err(|_| "array key length overflow")?;
                ir::Ty::Array {
                    len,
                    element: Arc::new(self.ty(facts, depth + 1)?),
                }
            }
            12 => ir::Ty::Slice {
                element: Arc::new(self.ty(facts, depth + 1)?),
            },
            13 | 14 => {
                let declaration = self.declaration()?;
                let args = self.types(facts, depth + 1)?;
                let ty = self
                    .context
                    .intern_nominal(declaration, &args, facts, self.budget)?;
                if !matches!((tag, &ty), (13, ir::Ty::Struct(_)) | (14, ir::Ty::Enum(_))) {
                    return Err("nominal graph key changed declaration kind".into());
                }
                ty
            }
            15 => ir::Ty::Dyn(Arc::new(ir::DynTy {
                principal: self.declaration()?,
                args: Arc::from([]),
                associated_types: Arc::from([]),
            })),
            16 => {
                let owner = self.declaration()?;
                let site = u32::try_from(self.word()?).map_err(|_| "opaque site key overflow")?;
                let args = self.types(facts, depth + 1)?;
                let region_count = self.count()?;
                let mut regions = Vec::new();
                for _ in 0..region_count {
                    regions.push(ir::SourceRegionId {
                        declaration: self.declaration()?,
                        index: u16::try_from(self.word()?)
                            .map_err(|_| "opaque capture key overflow")?,
                    });
                }
                let count = self.count()?;
                let mut bounds = Vec::new();
                for _ in 0..count {
                    let declaration = self.declaration()?;
                    let count = self.count()?;
                    let mut arguments = Vec::new();
                    for _ in 0..count {
                        arguments.push(match self.word()? {
                            0 => ir::OpaqueBoundArg::SelfType,
                            1 => ir::OpaqueBoundArg::Type(self.ty(facts, depth + 1)?),
                            _ => return Err("invalid opaque self argument tag".into()),
                        });
                    }
                    let count = self.count()?;
                    let mut equalities = Vec::new();
                    for _ in 0..count {
                        equalities.push((self.declaration()?, self.ty(facts, depth + 1)?));
                    }
                    bounds.push(ir::OpaqueBound::new(
                        declaration,
                        arguments.into(),
                        equalities.into(),
                    ));
                }
                ir::Ty::Opaque(Arc::new(ir::OpaqueTy::new(
                    owner,
                    args.into(),
                    site,
                    bounds.into(),
                    regions.into(),
                )))
            }
            17 => {
                let provider = self.declaration()?;
                let owner_args = self.types(facts, depth + 1)?;
                let ty = (self.closures)(provider, &owner_args)?;
                match &ty {
                    ir::Ty::Closure(closure)
                        if closure.provider() == provider
                            && closure.owner_args().len() == owner_args.len() => {}
                    _ => return Err("closure graph key replay changed its provider".into()),
                }
                ty
            }
            18 => {
                let declaration = self.declaration()?;
                let type_args = self.types(facts, depth + 1)?;
                ir::Ty::FnItem(Arc::new(ir::CallTarget {
                    declaration,
                    type_args: type_args.into(),
                }))
            }
            19 => {
                let inputs = self.types(facts, depth + 1)?;
                if inputs.len() > 32 {
                    return Err("function pointer parameter limit exceeded".into());
                }
                let output = self.ty(facts, depth + 1)?;
                ir::Ty::FnPointer(ir::FnPointerTy::new(inputs, output, self.budget)?)
            }
            _ => return Err("unsupported instance graph type tag".into()),
        };
        if self.at != end {
            return Err("instance key type frame length mismatch".into());
        }
        facts.validate_ty_metadata(&value)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(args: Vec<ir::Ty>) -> ir::CallTarget {
        ir::CallTarget {
            declaration: ir::DeclId {
                snapshot: ir::SnapshotId::standalone().unwrap(),
                index: 1,
            },
            type_args: args.into(),
        }
    }
    fn fixture() -> (frontend::PlanCtx, ir::PlanBudget, ir::DeclId) {
        let program = frontend::parse("fn item<T>(x:T)->T{x}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = (0..context.callable_count())
            .map(|i| context.callable_decl(i).unwrap())
            .find(|id| {
                matches!(
                    context.persistent_decl_path(*id).unwrap().item(),
                    frontend::PersistentDeclItem::Name("item")
                )
            })
            .unwrap();
        (context, budget, declaration)
    }
    #[test]
    fn canonical_decode_roundtrips_and_rejects_foreign_or_malformed_keys() {
        let (context, budget, declaration) = fixture();
        for value in [
            ir::Ty::Bool,
            ir::Ty::Int(ir::IntTy::I32),
            ir::Ty::Tuple(Arc::from([ir::Ty::Bool, ir::Ty::Int(ir::IntTy::U8)])),
            ir::Ty::FnPointer(Arc::new(ir::FnPointerTy {
                inputs: Arc::from([ir::Ty::Bool, ir::Ty::Int(ir::IntTy::U8)]),
                output: ir::Ty::Unit,
            })),
            ir::Ty::FnPointer(Arc::new(ir::FnPointerTy {
                inputs: Arc::from([]),
                output: ir::Ty::Never,
            })),
        ] {
            let target = ir::CallTarget {
                declaration,
                type_args: Arc::from([value]),
            };
            let words = encode(&target, &budget).unwrap();
            assert_eq!(target, decode(&words, &context, &budget).unwrap());
            for end in 0..words.len() {
                assert!(
                    decode(&words[..end], &context, &budget).is_err(),
                    "accepted truncated key at {end}"
                );
            }
            let mut trailing = words.clone();
            trailing.push(0);
            assert!(decode(&trailing, &context, &budget).is_err());
            let mut foreign = words.clone();
            foreign[1] = ir::SnapshotId::standalone().unwrap().graph_word();
            assert!(decode(&foreign, &context, &budget).is_err());
            let mut overflowing = words.clone();
            overflowing[5] = u64::MAX;
            assert!(decode(&overflowing, &context, &budget).is_err());
            let mut short_frame = words.clone();
            short_frame[5] = 0;
            if words[4] != 3 {
                assert!(decode(&short_frame, &context, &budget).is_err());
            }
        }
        let wrong_arity = ir::CallTarget {
            declaration,
            type_args: Arc::from([]),
        };
        assert!(decode(&encode(&wrong_arity, &budget).unwrap(), &context, &budget).is_err());
    }
    #[test]
    fn nominal_alias_coordinates_share_and_nested_frames_remain_bounded() {
        let program = frontend::parse("struct Pair<T>{value:T} type Alias=Pair<i32>; fn first(x:Pair<i32>){} fn second(x:Alias){} fn item<T>(x:T)->T{x} fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let named = |name| {
            (0..context.callable_count()).map(|i| context.callable_decl(i).unwrap()).find(|id| matches!(context.persistent_decl_path(*id).unwrap().item(), frontend::PersistentDeclItem::Name(found) if found == name)).unwrap()
        };
        let first = context.signature(named("first")).unwrap().signature.params[0].clone();
        let second = context.signature(named("second")).unwrap().signature.params[0].clone();
        let target = |ty| ir::CallTarget {
            declaration: named("item"),
            type_args: Arc::from([ty]),
        };
        let words = encode(&target(first.clone()), &budget).unwrap();
        assert_eq!(words, encode(&target(second), &budget).unwrap());
        assert_eq!(decode(&words, &context, &budget).unwrap(), target(first));
        for end in 0..words.len() {
            assert!(decode(&words[..end], &context, &budget).is_err());
        }
        let mut wrong_kind = words.clone();
        wrong_kind[4] = 14;
        assert!(decode(&wrong_kind, &context, &budget).is_err());
        let mut foreign_nominal = words.clone();
        foreign_nominal[6] = ir::SnapshotId::standalone().unwrap().graph_word();
        assert!(decode(&foreign_nominal, &context, &budget).is_err());
        let mut opaque = encode(&target(ir::Ty::Bool), &budget).unwrap();
        opaque[4] = 16;
        assert!(decode(&opaque, &context, &budget).is_err());
    }
    #[test]
    fn opaque_closure_actuals_replay_canonical_templates_and_reject_forgery() {
        let program = frontend::parse(
            "trait Marker{} impl<T> Marker for T{} fn owner<T>(x:T){let f=move||{let _y=x;};} fn take<T>(x:T)->impl Marker{x} fn main(){}",
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let (provider, owner) = (0..context.callable_count())
            .find_map(|index| {
                let provider = context.callable_decl(index).ok()?;
                context
                    .stored_closure_owner(provider)
                    .ok()
                    .flatten()
                    .map(|(owner, _)| (provider, owner))
            })
            .unwrap();
        let constants = context.prepare_body_constants(owner, &budget).unwrap();
        let constants = context
            .seal_body_constants(constants, Vec::new(), &budget)
            .unwrap();
        let (_, closures) = context
            .publish_callable_body_with_closures(owner, constants, &budget)
            .unwrap();
        let template = closures
            .iter()
            .find(|closure| closure.ty.provider() == provider)
            .unwrap()
            .ty
            .clone();
        let mut templates = ClosureTemplates::new();
        templates.insert(provider, template.clone());
        let missing = std::cell::RefCell::new(Vec::new());
        let replay = |provider, args: &[ir::Ty]| {
            replay_closure(&context, &templates, &missing, &budget, provider, args)
        };
        let inner = replay(provider, &[ir::Ty::Bool]).unwrap();
        let outer = replay(provider, &[inner]).unwrap();
        let take = (0..context.callable_count())
            .map(|i| context.callable_decl(i).unwrap())
            .find(|id| {
                matches!(
                    context.persistent_decl_path(*id).unwrap().item(),
                    frontend::PersistentDeclItem::Name("take")
                )
            })
            .unwrap();
        let ir::Ty::Opaque(schema) = context.signature(take).unwrap().signature.result else {
            panic!("fixture must declare an opaque return");
        };
        let opaque = Arc::new(ir::OpaqueTy::new(
            take,
            Arc::from([outer]),
            schema.site(),
            Arc::from(schema.bounds()),
            Arc::from(schema.captured_regions()),
        ));
        let words = encode_opaque(&opaque, &budget).unwrap();
        assert!(decode_opaque(&words, &context, &budget).is_err());
        assert!(
            decode_opaque_with_closures(&words, &context, &budget, &|p, a| {
                replay_closure(&context, &ClosureTemplates::new(), &missing, &budget, p, a)
            })
            .is_err()
        );
        assert_eq!(missing.borrow().as_slice(), &[provider]);
        assert_eq!(
            decode_opaque_with_closures(&words, &context, &budget, &replay).unwrap(),
            opaque
        );
        for end in 0..words.len() {
            assert!(
                decode_opaque_with_closures(&words[..end], &context, &budget, &replay).is_err()
            );
        }
        let mut trailing = words.clone();
        trailing.push(0);
        assert!(decode_opaque_with_closures(&trailing, &context, &budget, &replay).is_err());
        let mut foreign = words.clone();
        foreign[1] = ir::SnapshotId::standalone().unwrap().graph_word();
        assert!(decode_opaque_with_closures(&foreign, &context, &budget, &replay).is_err());
        // A valid closure with different owner actuals cannot replace the key's
        // requested closure, even though its provider and arity match.
        assert!(
            decode_opaque_with_closures(&words, &context, &budget, &|p, _| {
                replay(p, &[ir::Ty::Unit])
            })
            .is_err()
        );
        let concrete = match replay(provider, &[ir::Ty::Bool]).unwrap() {
            ir::Ty::Closure(value) => value,
            _ => unreachable!(),
        };
        let forged = ClosureTemplates::from([(provider, concrete)]);
        assert!(
            decode_opaque_with_closures(&words, &context, &budget, &|p, a| {
                replay_closure(&context, &forged, &missing, &budget, p, a)
            })
            .is_err()
        );
    }

    #[test]
    fn tags_boundaries_and_order_are_injective() {
        let budget = ir::PlanBudget::default();
        let mut original = target(vec![ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool]);
        let first = encode(&original, &budget).unwrap();
        original.type_args = Arc::from([ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)]);
        assert_ne!(first, encode(&original, &budget).unwrap());
        original.type_args = Arc::from([ir::Ty::Tuple(Arc::from([
            ir::Ty::Int(ir::IntTy::I32),
            ir::Ty::Bool,
        ]))]);
        assert_ne!(first, encode(&original, &budget).unwrap());
        let pointer = |inputs: Vec<ir::Ty>, output| {
            ir::Ty::FnPointer(Arc::new(ir::FnPointerTy {
                inputs: inputs.into(),
                output,
            }))
        };
        let mut signature = original.clone();
        signature.type_args = Arc::from([pointer(vec![ir::Ty::Bool], ir::Ty::Unit)]);
        let first = encode(&signature, &budget).unwrap();
        for changed in [
            pointer(vec![ir::Ty::Unit], ir::Ty::Bool),
            pointer(vec![], ir::Ty::Bool),
            pointer(vec![ir::Ty::Bool], ir::Ty::Bool),
            pointer(vec![ir::Ty::Bool, ir::Ty::Bool], ir::Ty::Unit),
        ] {
            signature.type_args = Arc::from([changed]);
            assert_ne!(first, encode(&signature, &budget).unwrap());
        }
        let mut other = original.clone();
        other.declaration.snapshot = ir::SnapshotId::standalone().unwrap();
        assert_ne!(
            encode(&original, &budget).unwrap(),
            encode(&other, &budget).unwrap()
        );
    }
    #[test]
    fn opaque_owner_site_actuals_and_captures_are_part_of_key() {
        let budget = ir::PlanBudget::default();
        let base = target(Vec::new());
        let opaque = |site, args: Vec<ir::Ty>, regions: Vec<ir::SourceRegionId>| {
            ir::Ty::Opaque(Arc::new(ir::OpaqueTy::new(
                base.declaration,
                args.into(),
                site,
                Arc::from([]),
                regions.into(),
            )))
        };
        let key = |value| {
            encode(
                &ir::CallTarget {
                    declaration: base.declaration,
                    type_args: Arc::from([value]),
                },
                &budget,
            )
            .unwrap()
        };
        let first = key(opaque(0, vec![ir::Ty::Bool], vec![]));
        assert_ne!(first, key(opaque(1, vec![ir::Ty::Bool], vec![])));
        assert_ne!(
            first,
            key(opaque(0, vec![ir::Ty::Int(ir::IntTy::U8)], vec![]))
        );
        assert_ne!(
            first,
            key(opaque(
                0,
                vec![ir::Ty::Bool],
                vec![ir::SourceRegionId {
                    declaration: base.declaration,
                    index: 0
                }]
            ))
        );
    }
}
