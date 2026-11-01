//! Cardinality comes from the original zero constructor and one append, not capacity.
use super::*;

type Issuer = crate::compiler::source::EmbeddedOriginalFile;

fn original_method<'a>(
    registry: &'a DeclRegistry,
    declaration: ir::DeclId,
    owner: ir::DeclId,
    receiver: ReceiverKind,
    issuer: &Issuer,
    budget: &ir::PlanBudget,
) -> Result<Option<&'a Function>, String> {
    budget.charge_work(1)?;
    let info = registry.declaration(declaration)?;
    let Some(associated) = &info.path.associated else {
        return Ok(None);
    };
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
    let Some(implementation_source) = implementation.original_source else {
        return Ok(None);
    };
    let Some(member) = function.name.segments().last() else {
        return Ok(None);
    };
    let names = registry.program.names.as_ref();
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
    if associated.nominal != owner
        || associated.receiver != receiver
        || info.receiver != receiver
        || method.receiver != receiver
        || info.path.module != function.module
        || info.visibility != function.visibility
        || !issuer.authorizes(names.session().sources(), implementation_source)
        || !issuer.authorizes(names.session().sources(), source)
        || implementation.type_params.len() != 1
        || !implementation.bounds.is_empty()
        || !function.type_params.is_empty()
        || !function.bounds.is_empty()
        || function.unsafety
        || function.static_mut
        || function.constness != Constness::Runtime
        || function.impl_params != 0
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
    {
        return Ok(None);
    }
    Ok(Some(function))
}

pub(super) fn peer(
    registry: &DeclRegistry,
    owner: ir::DeclId,
    member: AstIdent,
    receiver: ReceiverKind,
    issuer: &Issuer,
    budget: &ir::PlanBudget,
) -> Result<Option<ir::DeclId>, String> {
    let names = registry.program.names.as_ref();
    let mut selected = None;
    for declaration in registry.inherent_peers.get(&owner).into_iter().flatten() {
        budget.charge_work(1)?;
        let Some(function) = registry.source_callable(*declaration) else {
            continue;
        };
        let Some(actual) = function.name.segments().last() else {
            continue;
        };
        if !same(names, *actual, member, LookupClass::Member)? {
            continue;
        }
        if selected.is_some()
            || original_method(registry, *declaration, owner, receiver, issuer, budget)?.is_none()
        {
            return Ok(None);
        }
        selected = Some(*declaration);
    }
    Ok(selected)
}
fn integer(expression: &Expr, value: u128) -> bool {
    matches!(expression, Expr::Integer(value_) if value_.magnitude == value && value_.suffix == Some(ir::IntTy::Usize))
}
fn empty(block: &Block) -> bool {
    block.statements.is_empty() && block.tail.is_none()
}
fn binary(expression: &Expr, op: BinaryOp) -> Option<(&Expr, &Expr)> {
    match expression {
        Expr::Binary {
            op: actual,
            left,
            right,
            ..
        } if *actual == op => Some((left, right)),
        _ => None,
    }
}
fn receiver(expression: &Expr) -> bool {
    matches!(expression, Expr::Var(path) if path.receiver_occurrence().is_some())
}
fn receiver_place(names: &NameStore, place: &Place, expected: AstIdent) -> Result<bool, String> {
    Ok(matches!(place.local, LocalName::ReceiverOccurrence(_))
        && matches!(place.fields.as_slice(), [FieldSelector::Named(actual)] if same(names,*actual,expected,LookupClass::Member)?))
}

/// Default-deny, finite effect proof for reserve. No alias or address of `self`
/// is expressible in this sublanguage. Only the pointer/capacity fields can be
/// assigned, and the only calls are replayed original numeric allocator/query
/// functions. Therefore the original length field is unchanged on every return.
struct Reserve<'a> {
    registry: &'a DeclRegistry,
    pointer: AstIdent,
    length: AstIdent,
    capacity: AstIdent,
    budget: &'a ir::PlanBudget,
}
impl Reserve<'_> {
    fn expression(&self, expression: &Expr) -> Result<bool, String> {
        self.budget.charge_work(1)?;
        let names = self.registry.program.names.as_ref();
        Ok(match expression {
            Expr::Integer(_) | Expr::Bool(_) => true,
            Expr::Var(path) => path.receiver_occurrence().is_none() && path.segments().len() == 1,
            Expr::Field { .. } => {
                field(names, expression, self.pointer)?
                    || field(names, expression, self.length)?
                    || field(names, expression, self.capacity)?
            }
            Expr::Binary { left, right, .. } => self.expression(left)? && self.expression(right)?,
            Expr::Cast { value, target } => {
                matches!(target, Type::Int(ir::IntTy::Usize) | Type::RawPtr { .. })
                    && self.expression(value)?
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expression(condition)? && self.block(then_block)? && self.block(else_block)?
            }
            Expr::Panic(value) => {
                !value.preformatted && value.message == "memory allocation failed"
            }
            Expr::Apply { path, value } => {
                let (_, arguments) = path.application()?;
                let Expr::Call { name, .. } = value.as_ref() else {
                    return Ok(false);
                };
                arguments.iter().all(|ty| matches!(ty, Type::Param(_)))
                    && name_scope::path_key(names, &path.path(names)?, Namespace::Value)?
                        == name_scope::path_key(names, name, Namespace::Value)?
                    && self.expression(value)?
            }
            Expr::Call { name, args } => {
                let key = name_scope::path_key(names, name, Namespace::Value)?;
                let Some(declaration) = self.registry.functions.get(&key).copied() else {
                    return Ok(false);
                };
                let info = self.registry.declaration(declaration)?;
                let Some(source) = info.embedded_function.as_ref() else {
                    return Ok(false);
                };
                source.replay_admission(
                    &self.registry.program,
                    self.registry,
                    declaration,
                    self.budget,
                )?;
                let Some(callee) = self.registry.source_callable(declaration) else {
                    return Ok(false);
                };
                let Some(member) = callee.name.segments().last() else {
                    return Ok(false);
                };
                let span = names
                    .span(*member)
                    .map_err(native_adapter::name_diagnostic)?;
                let crate::compiler::expansion_span::SpanOrigin::Original(original) = names
                    .session()
                    .span(span)
                    .map_err(|e| e.to_string())?
                    .origin
                else {
                    return Ok(false);
                };
                // These numeric helpers have their own complete original source
                // replay. Neither accepts a reference to the sequence owner.
                let allocator = spelling(names, *member, "raw_realloc")?
                    && args.len() == 3
                    && callee
                        .params
                        .iter()
                        .all(|(_, ty)| matches!(ty, Type::Int(ir::IntTy::Usize)))
                    && callee.type_params.is_empty();
                let stride = spelling(names, *member, "element_stride")?
                    && args.is_empty()
                    && callee.params.is_empty()
                    && callee.type_params.len() == 1;
                if !(allocator || stride)
                    || !matches!(callee.result, Type::Int(ir::IntTy::Usize))
                    || !self
                        .registry
                        .program
                        .embedded_issuers
                        .iter()
                        .any(|issuer| issuer.authorizes(names.session().sources(), original))
                {
                    return Ok(false);
                }
                for argument in args {
                    if !self.expression(argument)? {
                        return Ok(false);
                    }
                }
                true
            }
            _ => false,
        })
    }
    fn block(&self, block: &Block) -> Result<bool, String> {
        let names = self.registry.program.names.as_ref();
        for statement in &block.statements {
            self.budget.charge_work(1)?;
            let valid = match statement {
                Stmt::Let {
                    name: BindingName::Named(_),
                    ty: Type::Infer,
                    value,
                    ..
                } => self.expression(value)?,
                Stmt::Assign {
                    name: LocalName::Named(_),
                    value,
                } => self.expression(value)?,
                Stmt::AssignPlace { place, value, .. } => {
                    (receiver_place(names, place, self.pointer)?
                        || receiver_place(names, place, self.capacity)?)
                        && self.expression(value)?
                }
                Stmt::Expr(value) | Stmt::ImplicitExpr(value) => self.expression(value)?,
                Stmt::Return(None) => true,
                _ => false,
            };
            if !valid {
                return Ok(false);
            }
        }
        match &block.tail {
            Some(value) => self.expression(value),
            None => Ok(true),
        }
    }
}

pub(super) fn linked(
    registry: &DeclRegistry,
    constructor: ir::DeclId,
    owner: ir::DeclId,
    append_member: AstIdent,
    issuer: &Issuer,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let names = registry.program.names.as_ref();
    let Some(constructor) = original_method(
        registry,
        constructor,
        owner,
        ReceiverKind::None,
        issuer,
        budget,
    )?
    else {
        return Ok(false);
    };
    let owner_info = registry.declaration(owner)?;
    let Some(owner_source) = registry.program.structs.get(owner_info.source_index) else {
        return Ok(false);
    };
    let [pointer, length, capacity] = owner_source.fields.as_slice() else {
        return Ok(false);
    };
    let (FieldName::Named(pointer), FieldName::Named(length), FieldName::Named(capacity)) =
        (&pointer.name, &length.name, &capacity.name)
    else {
        return Ok(false);
    };
    let [(BindingName::Named(capacity_parameter), Type::Int(ir::IntTy::Usize))] =
        constructor.params.as_slice()
    else {
        return Ok(false);
    };
    let [
        Stmt::Let {
            name: BindingName::Named(local),
            mutable: true,
            ty: Type::Infer,
            value:
                Expr::Struct {
                    name: nominal,
                    fields,
                },
        },
        Stmt::Expr(Expr::MethodCall {
            receiver: reserve_receiver,
            name: reserve_member,
            type_args,
            args,
        }),
    ] = constructor.body.statements.as_slice()
    else {
        return Ok(false);
    };
    let [reserve_argument] = args.as_slice() else {
        return Ok(false);
    };
    let [
        (pointer_field, pointer_zero),
        (length_field, length_zero),
        (capacity_field, capacity_zero),
    ] = fields.as_slice()
    else {
        return Ok(false);
    };
    let Expr::Cast {
        value: pointer_value,
        target: Type::RawPtr {
            kind: ir::BorrowKind::Mut,
            pointee,
        },
    } = pointer_zero
    else {
        return Ok(false);
    };
    let Type::Apply(result) = &constructor.result else {
        return Ok(false);
    };
    let (_, result_arguments) = result.application()?;
    let [Type::Param(result_parameter)] = result_arguments else {
        return Ok(false);
    };
    let Type::Param(pointer_parameter) = pointee.as_ref() else {
        return Ok(false);
    };
    if registry.nominals.get(&name_scope::path_key(
        names,
        &result.path(names)?,
        Namespace::Type,
    )?) != Some(&owner)
        || !same(
            names,
            *result_parameter,
            *pointer_parameter,
            LookupClass::TypeParameter,
        )?
    {
        return Ok(false);
    }
    let Some(tail) = constructor.body.tail.as_deref() else {
        return Ok(false);
    };
    let key = name_scope::path_key(names, nominal, Namespace::Type)?;
    if registry.nominals.get(&key) != Some(&owner)
        || !same(names, *pointer_field, *pointer, LookupClass::Member)?
        || !same(names, *length_field, *length, LookupClass::Member)?
        || !same(names, *capacity_field, *capacity, LookupClass::Member)?
        || !integer(pointer_value, 0)
        || !integer(length_zero, 0)
        || !integer(capacity_zero, 0)
        || !matches!(pointee.as_ref(), Type::Param(_))
        || !type_args.is_empty()
        || !var(names, reserve_receiver, *local)?
        || !var(names, reserve_argument, *capacity_parameter)?
        || !var(names, tail, *local)?
        || !spelling(names, *reserve_member, "reserve")?
    {
        return Ok(false);
    }
    let mut_receiver = ReceiverKind::Borrowed {
        kind: ir::BorrowKind::Mut,
    };
    let Some(reserve_declaration) = peer(
        registry,
        owner,
        *reserve_member,
        mut_receiver,
        issuer,
        budget,
    )?
    else {
        return Ok(false);
    };
    let Some(reserve) = original_method(
        registry,
        reserve_declaration,
        owner,
        mut_receiver,
        issuer,
        budget,
    )?
    else {
        return Ok(false);
    };
    if !matches!(
        reserve.params.as_slice(),
        [(BindingName::Named(_), Type::Int(ir::IntTy::Usize))]
    ) || !matches!(reserve.result, Type::Unit)
        || reserve.body.tail.is_some()
        || !(Reserve {
            registry,
            pointer: *pointer,
            length: *length,
            capacity: *capacity,
            budget,
        })
        .block(&reserve.body)?
    {
        return Ok(false);
    }
    let Some(append_declaration) =
        peer(registry, owner, append_member, mut_receiver, issuer, budget)?
    else {
        return Ok(false);
    };
    let Some(append) = original_method(
        registry,
        append_declaration,
        owner,
        mut_receiver,
        issuer,
        budget,
    )?
    else {
        return Ok(false);
    };
    let [(BindingName::Named(value), Type::Param(append_parameter))] = append.params.as_slice()
    else {
        return Ok(false);
    };
    if !same(
        names,
        *append_parameter,
        *pointer_parameter,
        LookupClass::TypeParameter,
    )? {
        return Ok(false);
    }
    let [
        Stmt::ImplicitExpr(Expr::If {
            condition,
            then_block,
            else_block,
        }),
        Stmt::Let {
            name: BindingName::Named(destination),
            mutable: false,
            ty: Type::Infer,
            value:
                Expr::Call {
                    name: slot,
                    args: slot_args,
                },
        },
        Stmt::ImplicitExpr(Expr::Unsafe(store)),
        Stmt::AssignOp {
            place: increment,
            op: BinaryOp::Add,
            value: one,
            ..
        },
    ] = append.body.statements.as_slice()
    else {
        return Ok(false);
    };
    let Some((full_length, full_capacity)) = binary(condition, BinaryOp::Eq) else {
        return Ok(false);
    };
    let [
        Stmt::Expr(Expr::MethodCall {
            receiver: growth_receiver,
            name: growth_member,
            type_args: growth_types,
            args: growth_args,
        }),
    ] = then_block.statements.as_slice()
    else {
        return Ok(false);
    };
    let [growth_argument] = growth_args.as_slice() else {
        return Ok(false);
    };
    let [base, index] = slot_args.as_slice() else {
        return Ok(false);
    };
    let [
        Stmt::AssignPlace {
            place: stored,
            value: stored_value,
            ..
        },
    ] = store.statements.as_slice()
    else {
        return Ok(false);
    };
    if !matches!(append.result, Type::Unit)
        || append.body.tail.is_some()
        || !field(names, full_length, *length)?
        || !field(names, full_capacity, *capacity)?
        || !empty(else_block)
        || then_block.tail.is_some()
        || !receiver(growth_receiver)
        || !growth_types.is_empty()
        || !integer(growth_argument, 1)
        || !same(names, *growth_member, *reserve_member, LookupClass::Member)?
        || !field(names, base, *pointer)?
        || !field(names, index, *length)?
        || store.tail.is_some()
        || !matches!(stored.local,LocalName::Named(actual) if same(names,actual,*destination,LookupClass::Local)?)
        || !matches!(stored.fields.as_slice(), [FieldSelector::Deref])
        || !var(names, stored_value, *value)?
        || !receiver_place(names, increment, *length)?
        || !integer(one, 1)
    {
        return Ok(false);
    }
    let key = name_scope::path_key(names, slot, Namespace::Value)?;
    let Some(slot_declaration) = registry.functions.get(&key).copied() else {
        return Ok(false);
    };
    Ok(super::super::issue_slot(registry, slot_declaration, budget)?.is_some())
}
