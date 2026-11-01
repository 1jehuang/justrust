//! Original native singleton construction and indexed-slot recipes.
//! These roles do not authorize an allocation or a slot in a caller by themselves.
use super::*;

mod seed_source;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compiler) enum NativeElementRole {
    Seed,
    SharedIndex,
    MutIndex,
}
#[derive(Clone, Debug)]
pub struct NativeElementSource {
    declaration: ir::DeclId,
    role: NativeElementRole,
    owner: Option<ir::DeclId>,
    issuer: crate::compiler::source::EmbeddedOriginalFile,
    _source: crate::compiler::token_session::OriginalSpan,
    _slot: Option<(ir::DeclId, Arc<NativeSequenceSlot>)>,
}
impl NativeElementSource {
    pub(in crate::compiler) fn role_for(
        &self,
        declaration: ir::DeclId,
        owner: &ir::StructTy,
    ) -> Option<NativeElementRole> {
        (self.declaration == declaration
            && self.owner.is_none_or(|id| id == owner.def().source_decl)
            && self
                ._slot
                .as_ref()
                .is_none_or(|(declaration, slot)| slot.matches_owner(*declaration, owner))
            && owner.native_sequence_binding().is_some_and(|binding| {
                binding.matches_declaration(owner.def())
                    && binding._issuer.same_issuer(&self.issuer)
            }))
        .then_some(self.role)
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
fn var(names: &NameStore, expr: &Expr, expected: AstIdent) -> Result<bool, String> {
    Ok(
        matches!(expr, Expr::Var(path) if path.segments().len() == 1 && same(names, path.segments()[0], expected, LookupClass::Local)?),
    )
}
fn field(names: &NameStore, expr: &Expr, expected: AstIdent) -> Result<bool, String> {
    Ok(
        matches!(expr, Expr::Field { base, field: FieldSelector::Named(actual) }
        if matches!(base.as_ref(), Expr::Var(path) if path.receiver_occurrence().is_some())
        && same(names, *actual, expected, LookupClass::Member)?),
    )
}
fn spelling(names: &NameStore, name: AstIdent, expected: &str) -> Result<bool, String> {
    Ok(names
        .spelling(name)
        .map_err(native_adapter::name_diagnostic)?
        == expected)
}
fn seed(
    function: &Function,
    names: &NameStore,
    registry: &DeclRegistry,
    issuer: &crate::compiler::source::EmbeddedOriginalFile,
    budget: &ir::PlanBudget,
) -> Result<Option<ir::DeclId>, String> {
    let [
        (BindingName::Named(first), Type::Param(parameter)),
        (BindingName::Named(capacity), Type::Int(ir::IntTy::Usize)),
    ] = function.params.as_slice()
    else {
        return Ok(None);
    };
    if function.type_params.len() != 1
        || !same(
            names,
            function.type_params[0],
            *parameter,
            LookupClass::TypeParameter,
        )?
    {
        return Ok(None);
    }
    let [
        Stmt::Let {
            name: BindingName::Named(owner),
            mutable: true,
            ty: annotation,
            value:
                Expr::AssociatedCall {
                    owner: constructor_owner,
                    name: member,
                    type_args: constructor_actuals,
                    args,
                },
        },
        Stmt::Expr(Expr::MethodCall {
            receiver,
            name: append,
            type_args,
            args: values,
        }),
    ] = function.body.statements.as_slice()
    else {
        return Ok(None);
    };
    let [capacity_arg] = args.as_slice() else {
        return Ok(None);
    };
    let [first_arg] = values.as_slice() else {
        return Ok(None);
    };
    // associated_path records the unwritten Vec impl argument as Infer.
    // The authored annotation/result below independently fix that one T.
    let Type::Apply(constructor_owner) = constructor_owner else {
        return Ok(None);
    };
    let (_, owner_actuals) = constructor_owner.application()?;
    if !matches!(owner_actuals, [Type::Infer]) {
        return Ok(None);
    }
    for segment in &constructor_owner.segments {
        budget.charge_work(1)?;
        if !segment.region_args.is_empty() || !segment.associated_types.is_empty() {
            return Ok(None);
        }
    }
    let constructor_owner = constructor_owner.path(names)?;
    if !constructor_actuals.is_empty() {
        return Ok(None);
    }
    let Some(tail) = function.body.tail.as_deref() else {
        return Ok(None);
    };
    let key = name_scope::path_key(names, &constructor_owner, Namespace::Type)?;
    let Some(nominal) = registry.nominals.get(&key).copied() else {
        return Ok(None);
    };
    let Some(constructor) = seed_source::peer(
        registry,
        nominal,
        *member,
        ReceiverKind::None,
        issuer,
        budget,
    )?
    else {
        return Ok(None);
    };
    for ty in [&function.result, annotation] {
        let Type::Apply(path) = ty else {
            return Ok(None);
        };
        let (_, arguments) = path.application()?;
        let [Type::Param(actual)] = arguments else {
            return Ok(None);
        };
        let key = name_scope::path_key(names, &path.path(names)?, Namespace::Type)?;
        if registry.nominals.get(&key) != Some(&nominal)
            || !same(names, *actual, *parameter, LookupClass::TypeParameter)?
        {
            return Ok(None);
        }
    }
    if spelling(names, *member, "with_capacity")?
        && spelling(names, *append, "push")?
        && type_args.is_empty()
        && var(names, capacity_arg, *capacity)?
        && var(names, receiver, *owner)?
        && var(names, first_arg, *first)?
        && var(names, tail, *owner)?
    {
        if !seed_source::linked(registry, constructor, nominal, *append, issuer, budget)? {
            return Ok(None);
        }
        Ok(Some(nominal))
    } else {
        Ok(None)
    }
}
fn indexed<'a>(
    function: &'a Function,
    names: &NameStore,
    pointer: AstIdent,
    length: AstIdent,
    shared: bool,
) -> Result<Option<&'a AstPath>, String> {
    let [(BindingName::Named(index), Type::Int(ir::IntTy::Usize))] = function.params.as_slice()
    else {
        return Ok(None);
    };
    let [
        Stmt::ImplicitExpr(Expr::If {
            condition,
            then_block,
            else_block,
        }),
        Stmt::Let {
            name: BindingName::Named(slot),
            mutable: false,
            ty: Type::Infer,
            value,
        },
    ] = function.body.statements.as_slice()
    else {
        return Ok(None);
    };
    let Expr::Binary {
        op: BinaryOp::Ge,
        left,
        right,
        ..
    } = condition.as_ref()
    else {
        return Ok(None);
    };
    if !var(names, left, *index)?
        || !field(names, right, length)?
        || !else_block.statements.is_empty()
        || else_block.tail.is_some()
        || then_block.tail.is_some()
        || !matches!(then_block.statements.as_slice(), [Stmt::Expr(Expr::Panic(p))] if !p.preformatted && p.message == "index out of bounds")
    {
        return Ok(None);
    }
    let value = if shared {
        let Expr::Cast {
            value,
            target:
                Type::RawPtr {
                    kind: ir::BorrowKind::Shared,
                    ..
                },
        } = value
        else {
            return Ok(None);
        };
        value.as_ref()
    } else {
        value
    };
    let Expr::Call { name, args } = value else {
        return Ok(None);
    };
    let [base, subscript] = args.as_slice() else {
        return Ok(None);
    };
    if !field(names, base, pointer)? || !var(names, subscript, *index)? {
        return Ok(None);
    }
    let Some(Expr::Unsafe(block)) = function.body.tail.as_deref() else {
        return Ok(None);
    };
    if !block.statements.is_empty() {
        return Ok(None);
    }
    let expected = if shared {
        ir::BorrowKind::Shared
    } else {
        ir::BorrowKind::Mut
    };
    let Some(Expr::Borrow { kind, place }) = block.tail.as_deref() else {
        return Ok(None);
    };
    let LocalName::Named(actual) = place.local else {
        return Ok(None);
    };
    if *kind != expected
        || !same(names, actual, *slot, LookupClass::Local)?
        || !matches!(place.fields.as_slice(), [FieldSelector::Deref])
    {
        return Ok(None);
    }
    Ok(Some(name))
}

pub(in crate::compiler::frontend) fn issue(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Option<Arc<NativeElementSource>>, String> {
    budget.charge_work(128)?;
    let Some(function) = registry.source_callable(declaration) else {
        return Ok(None);
    };
    let info = registry.declaration(declaration)?;
    if function.unsafety
        || function.static_mut
        || function.constness != Constness::Runtime
        || function.impl_params != 0
        || !function.bounds.is_empty()
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
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
    let mut issuer = None;
    for candidate in registry.program.embedded_issuers.iter() {
        budget.charge_work(1)?;
        if candidate.authorizes(names.session().sources(), source) {
            issuer = Some(candidate.clone());
            break;
        }
    }
    let Some(issuer) = issuer else {
        return Ok(None);
    };
    let seed_owner =
        if info.receiver == ReceiverKind::None && spelling(names, *member, "__jr_vec_seed")? {
            seed(function, names, registry, &issuer, budget)?
        } else {
            None
        };
    let (role, owner, slot) = if let Some(owner) = seed_owner {
        let key = name_scope::path_key(names, &function.name, Namespace::Value)?;
        if registry.functions.get(&key) != Some(&declaration)
            || info.path.associated.is_some()
            || info.param_env.arity() != 1
        {
            return Ok(None);
        }
        let Some(original) = info.embedded_function.as_ref() else {
            return Ok(None);
        };
        original.replay_admission(&registry.program, registry, declaration, budget)?;
        (NativeElementRole::Seed, Some(owner), None)
    } else {
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
        let Some(associated) = &info.path.associated else {
            return Ok(None);
        };
        let Some(original) = implementation.original_source else {
            return Ok(None);
        };
        if !issuer.authorizes(names.session().sources(), original)
            || method.receiver != info.receiver
            || associated.receiver != info.receiver
            || implementation.type_params.len() != 1
            || !implementation.bounds.is_empty()
            || !function.type_params.is_empty()
        {
            return Ok(None);
        }
        let owner_info = registry.declaration(associated.nominal)?;
        let Some(owner) = registry.program.structs.get(owner_info.source_index) else {
            return Ok(None);
        };
        let [pointer, length, _] = owner.fields.as_slice() else {
            return Ok(None);
        };
        let (FieldName::Named(pointer), FieldName::Named(length)) = (&pointer.name, &length.name)
        else {
            return Ok(None);
        };
        let (role, path) = if info.receiver
            == (ReceiverKind::Borrowed {
                kind: ir::BorrowKind::Shared,
            })
            && spelling(names, *member, "index")?
        {
            let [(BindingName::Named(index), Type::Int(ir::IntTy::Usize))] =
                function.params.as_slice()
            else {
                return Ok(None);
            };
            let Some(Expr::MethodCall {
                receiver,
                name,
                type_args,
                args,
            }) = function.body.tail.as_deref()
            else {
                return Ok(None);
            };
            if !function.body.statements.is_empty()
                || !matches!(receiver.as_ref(), Expr::Var(p) if p.receiver_occurrence().is_some())
                || !spelling(names, *name, "get_ref")?
                || !type_args.is_empty()
                || args.len() != 1
                || !var(names, &args[0], *index)?
            {
                return Ok(None);
            }
            let mut delegate = None;
            for candidate in registry
                .inherent_peers
                .get(&associated.nominal)
                .into_iter()
                .flatten()
            {
                budget.charge_work(1)?;
                let Some(original) = registry.source_callable(*candidate) else {
                    continue;
                };
                let Some(actual) = original.name.segments().last() else {
                    continue;
                };
                if same(names, *actual, *name, LookupClass::Member)? {
                    if delegate.is_some() {
                        return Ok(None);
                    }
                    delegate = issue(registry, *candidate, budget)?;
                }
            }
            let Some(delegate) = delegate else {
                return Ok(None);
            };
            if delegate.role != NativeElementRole::SharedIndex
                || delegate.owner != Some(associated.nominal)
                || !delegate.issuer.same_issuer(&issuer)
            {
                return Ok(None);
            }
            budget.charge_nodes(1)?;
            budget.retain_bytes(std::mem::size_of::<NativeElementSource>() + 32)?;
            return Ok(Some(Arc::new(NativeElementSource {
                declaration,
                role: NativeElementRole::SharedIndex,
                owner: Some(associated.nominal),
                issuer,
                _source: source,
                _slot: delegate._slot.clone(),
            })));
        } else if info.receiver
            == (ReceiverKind::Borrowed {
                kind: ir::BorrowKind::Mut,
            })
            && spelling(names, *member, "index_mut")?
        {
            let Some(path) = indexed(function, names, *pointer, *length, false)? else {
                return Ok(None);
            };
            (NativeElementRole::MutIndex, path)
        } else if info.receiver
            == (ReceiverKind::Borrowed {
                kind: ir::BorrowKind::Shared,
            })
            && spelling(names, *member, "get_ref")?
        {
            let Some(path) = indexed(function, names, *pointer, *length, true)? else {
                return Ok(None);
            };
            (NativeElementRole::SharedIndex, path)
        } else {
            return Ok(None);
        };
        let key = name_scope::path_key(names, path, Namespace::Value)?;
        let Some(slot_decl) = registry.functions.get(&key).copied() else {
            return Ok(None);
        };
        let Some(slot) = super::issue_slot(registry, slot_decl, budget)? else {
            return Ok(None);
        };
        (role, Some(associated.nominal), Some((slot_decl, slot)))
    };
    budget.charge_nodes(1)?;
    budget.retain_bytes(std::mem::size_of::<NativeElementSource>() + 32)?;
    Ok(Some(Arc::new(NativeElementSource {
        declaration,
        role,
        owner,
        issuer,
        _source: source,
        _slot: slot,
    })))
}
