//! Source-shape half of the paired native exchange receipt. This module does
//! not authorize a raw read or write by itself.
use super::*;

#[derive(Debug)]
pub struct NativeSequenceSwap {
    declaration: ir::DeclId,
    owner: ir::DeclId,
    slot: ir::DeclId,
    _source: crate::compiler::token_session::OriginalSpan,
    issuer: crate::compiler::source::EmbeddedOriginalFile,
    _slot: Arc<NativeSequenceSlot>,
}
impl NativeSequenceSwap {
    pub(in crate::compiler) fn matches_owner(
        &self,
        declaration: ir::DeclId,
        owner: &ir::StructTy,
        slot: ir::DeclId,
    ) -> bool {
        self.declaration == declaration
            && self.owner == owner.def().source_decl
            && self.slot == slot
            && self._slot.matches_owner(slot, owner)
            && owner.native_sequence_binding().is_some_and(|binding| {
                binding.matches_declaration(owner.def())
                    && binding._issuer.same_issuer(&self.issuer)
            })
    }
}
fn same(names: &NameStore, a: AstIdent, b: AstIdent, class: LookupClass) -> Result<bool, String> {
    Ok(names
        .key(a, class)
        .map_err(native_adapter::name_diagnostic)?
        == names
            .key(b, class)
            .map_err(native_adapter::name_diagnostic)?)
}
fn var(names: &NameStore, expression: &Expr, name: AstIdent) -> Result<bool, String> {
    match expression {
        Expr::Var(path) if path.segments().len() == 1 => {
            same(names, path.segments()[0], name, LookupClass::Local)
        }
        _ => Ok(false),
    }
}
fn receiver_field(names: &NameStore, expression: &Expr, field: AstIdent) -> Result<bool, String> {
    let Expr::Field {
        base,
        field: FieldSelector::Named(actual),
    } = expression
    else {
        return Ok(false);
    };
    Ok(
        matches!(base.as_ref(), Expr::Var(path) if path.receiver_occurrence().is_some())
            && same(names, *actual, field, LookupClass::Member)?,
    )
}
fn binary(expression: &Expr, expected: BinaryOp) -> Option<(&Expr, &Expr)> {
    match expression {
        Expr::Binary {
            op, left, right, ..
        } if *op == expected => Some((left, right)),
        _ => None,
    }
}
fn empty(block: &Block) -> bool {
    block.statements.is_empty() && block.tail.is_none()
}
fn deref_var(names: &NameStore, expression: &Expr, name: AstIdent) -> Result<bool, String> {
    match expression {
        Expr::Deref { value } => var(names, value, name),
        _ => Ok(false),
    }
}
fn store(names: &NameStore, place: &Place, name: AstIdent) -> Result<bool, String> {
    let LocalName::Named(actual) = place.local else {
        return Ok(false);
    };
    Ok(matches!(place.fields.as_slice(), [FieldSelector::Deref])
        && same(names, actual, name, LookupClass::Local)?)
}
/// Match the entire operation, including the guards and read-before-write order.
/// Binding identities are compared hygienically, never by printed spelling.
fn recipe<'a>(
    function: &'a Function,
    names: &NameStore,
    pointer: AstIdent,
    length: AstIdent,
) -> Result<Option<(&'a AstPath, &'a AstPath)>, String> {
    let [
        (BindingName::Named(a), Type::Int(ir::IntTy::Usize)),
        (BindingName::Named(b), Type::Int(ir::IntTy::Usize)),
    ] = function.params.as_slice()
    else {
        return Ok(None);
    };
    if function.param_mut.as_slice() != [false, false]
        || !matches!(function.result, Type::Unit)
        || function.unsafety
        || function.static_mut
        || function.constness != Constness::Runtime
        || function.impl_params != 0
        || !function.bounds.is_empty()
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
    {
        return Ok(None);
    }
    let [
        Stmt::ImplicitExpr(Expr::If {
            condition: bounds,
            then_block: panic,
            else_block: bounds_else,
        }),
        Stmt::ImplicitExpr(Expr::If {
            condition: equal,
            then_block: early,
            else_block: equal_else,
        }),
        Stmt::Let {
            name: BindingName::Named(pa),
            mutable: false,
            ty: Type::Infer,
            value:
                Expr::Call {
                    name: slot_a,
                    args: args_a,
                },
        },
        Stmt::Let {
            name: BindingName::Named(pb),
            mutable: false,
            ty: Type::Infer,
            value:
                Expr::Call {
                    name: slot_b,
                    args: args_b,
                },
        },
    ] = function.body.statements.as_slice()
    else {
        return Ok(None);
    };
    let Some((bound_a, bound_b)) = binary(bounds, BinaryOp::Or) else {
        return Ok(None);
    };
    let Some((actual_a, len_a)) = binary(bound_a, BinaryOp::Ge) else {
        return Ok(None);
    };
    let Some((actual_b, len_b)) = binary(bound_b, BinaryOp::Ge) else {
        return Ok(None);
    };
    let Some((equal_a, equal_b)) = binary(equal, BinaryOp::Eq) else {
        return Ok(None);
    };
    if !var(names, actual_a, *a)?
        || !var(names, actual_b, *b)?
        || !receiver_field(names, len_a, length)?
        || !receiver_field(names, len_b, length)?
        || !var(names, equal_a, *a)?
        || !var(names, equal_b, *b)?
        || !empty(bounds_else)
        || !empty(equal_else)
        || panic.tail.is_some()
        || !matches!(panic.statements.as_slice(), [Stmt::Expr(Expr::Panic(value))] if !value.preformatted && value.message == "swap index out of bounds")
        || early.tail.is_some()
        || !matches!(early.statements.as_slice(), [Stmt::Return(None)])
    {
        return Ok(None);
    }
    let [base_a, index_a] = args_a.as_slice() else {
        return Ok(None);
    };
    let [base_b, index_b] = args_b.as_slice() else {
        return Ok(None);
    };
    if !receiver_field(names, base_a, pointer)?
        || !receiver_field(names, base_b, pointer)?
        || !var(names, index_a, *a)?
        || !var(names, index_b, *b)?
    {
        return Ok(None);
    }
    let Some(Expr::Unsafe(exchange)) = function.body.tail.as_deref() else {
        return Ok(None);
    };
    let [
        Stmt::Let {
            name: BindingName::Named(tmp),
            mutable: false,
            ty: Type::Infer,
            value: first,
        },
        Stmt::AssignPlace {
            place: to_a,
            value: from_b,
            ..
        },
        Stmt::AssignPlace {
            place: to_b,
            value: from_tmp,
            ..
        },
    ] = exchange.statements.as_slice()
    else {
        return Ok(None);
    };
    if exchange.tail.is_some()
        || !deref_var(names, first, *pa)?
        || !store(names, to_a, *pa)?
        || !deref_var(names, from_b, *pb)?
        || !store(names, to_b, *pb)?
        || !var(names, from_tmp, *tmp)?
    {
        return Ok(None);
    }
    // Shadowing or binding aliasing must not change the staged exchange.
    let bindings = [*a, *b, *pa, *pb, *tmp];
    for (index, a) in bindings.iter().enumerate() {
        for b in &bindings[..index] {
            if same(names, *a, *b, LookupClass::Local)? {
                return Ok(None);
            }
        }
    }
    Ok(Some((slot_a, slot_b)))
}

pub(in crate::compiler::frontend) fn issue_swap(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Option<Arc<NativeSequenceSwap>>, String> {
    // The accepted shape has fixed size. Charge before any shape traversal or
    // name comparison, including all fifteen pairwise binding comparisons.
    budget.charge_work(256)?;
    let Some(position) = registry.function_position(declaration) else {
        return Ok(None);
    };
    let Some(CallableSource::Associated {
        implementation,
        item,
    }) = registry.callable_sources.get(position).copied()
    else {
        return Ok(None);
    };
    let implementation = &registry.program.impls[implementation];
    let method = &implementation.items[item];
    let function = &method.function;
    let info = registry.declaration(declaration)?;
    let Some(associated) = &info.path.associated else {
        return Ok(None);
    };
    if associated.trait_decl.is_some()
        || associated.trait_item.is_some()
        || method.receiver
            != (ReceiverKind::Borrowed {
                kind: ir::BorrowKind::Mut,
            })
        || associated.receiver != method.receiver
        || info.receiver != method.receiver
        || info.path.module != function.module
        || info.visibility != function.visibility
        || implementation.module != function.module
        || implementation.trait_path.is_some()
        || !implementation.bounds.is_empty()
        || implementation.type_params.len() != 1
        || !function.type_params.is_empty()
        || registry.declaration(declaration)?.param_env.arity() != 0
    {
        return Ok(None);
    }
    let names = registry.program.names.as_ref();
    let Some(member) = function.name.segments().last() else {
        return Ok(None);
    };
    let span = names
        .span(*member)
        .map_err(native_adapter::name_diagnostic)?;
    let crate::compiler::expansion_span::SpanOrigin::Original(source) = names
        .session()
        .span(span)
        .map_err(|e| e.to_string())?
        .origin
    else {
        return Ok(None);
    };
    let Some(impl_source) = implementation.original_source else {
        return Ok(None);
    };
    let mut issuer = None;
    for candidate in registry.program.embedded_issuers.iter() {
        budget.charge_work(1)?;
        if candidate.authorizes(names.session().sources(), source)
            && candidate.authorizes(names.session().sources(), impl_source)
        {
            issuer = Some(candidate);
            break;
        }
    }
    let Some(issuer) = issuer else {
        return Ok(None);
    };
    let owner_info = registry.declaration(associated.nominal)?;
    let Some(owner) = registry.program.structs.get(owner_info.source_index) else {
        return Ok(None);
    };
    let [pointer, length, _capacity] = owner.fields.as_slice() else {
        return Ok(None);
    };
    let (FieldName::Named(pointer), FieldName::Named(length)) = (&pointer.name, &length.name)
    else {
        return Ok(None);
    };
    let Some((a, b)) = recipe(function, names, *pointer, *length)? else {
        return Ok(None);
    };
    budget.charge_work(
        a.segments()
            .len()
            .saturating_add(b.segments().len())
            .saturating_add(1),
    )?;
    let key = name_scope::path_key(names, a, Namespace::Value)?;
    if key != name_scope::path_key(names, b, Namespace::Value)? {
        return Ok(None);
    }
    let Some(slot) = registry.functions.get(&key).copied() else {
        return Ok(None);
    };
    let Some(slot_receipt) = super::slot::issue_slot(registry, slot, budget)? else {
        return Ok(None);
    };
    budget.charge_nodes(1)?;
    budget.retain_bytes(
        std::mem::size_of::<NativeSequenceSwap>() + 2 * std::mem::size_of::<usize>(),
    )?;
    Ok(Some(Arc::new(NativeSequenceSwap {
        declaration,
        owner: associated.nominal,
        slot,
        _source: source,
        issuer: issuer.clone(),
        _slot: slot_receipt,
    })))
}
