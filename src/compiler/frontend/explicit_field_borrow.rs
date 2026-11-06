//! Authored local field borrows through one canonical Deref adjustment.
use super::*;

impl type_check::Checker<'_> {
    pub(super) fn explicit_field_stored(
        &self,
        nominal: &ir::StructTy,
        field: &FieldSelector,
    ) -> Result<bool, String> {
        let plan = self.signatures.plan()?;
        plan.budget.charge_work(1)?;
        let schema = plan
            .nominal_schemas
            .get(&nominal.def().source_decl)
            .ok_or("missing nominal source schema")?;
        let NominalSchema::Struct(source) = schema.as_ref() else {
            return Err("contract: struct has a non-struct source schema".into());
        };
        if !matches!(
            (source.shape, field),
            (StructShape::Record, FieldSelector::Named(_))
                | (StructShape::Tuple, FieldSelector::Tuple(_))
        ) {
            return Ok(false);
        }
        self.has_member(nominal.def().source_decl, field)
    }

    fn explicit_field_projection_push(
        &self,
        projection: &mut Vec<ir::Projection>,
        step: ir::Projection,
    ) -> Result<(), String> {
        if projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("place projection nesting exceeds limit128".into());
        }
        self.charge_node(std::mem::size_of::<ir::Projection>())?;
        projection
            .try_reserve_exact(1)
            .map_err(|_| "projected borrow allocation resource limit")?;
        projection.push(step);
        Ok(())
    }

    pub(super) fn explicit_field_borrow_owner(
        &self,
        place: &Place,
    ) -> Result<Option<ir::Ty>, String> {
        let Some(first) = place.fields.first() else {
            return Ok(None);
        };
        if !matches!(first, FieldSelector::Named(_) | FieldSelector::Tuple(_)) {
            return Ok(None);
        }
        let local = self.lookup(place.local)?;
        let owner = self.locals[local.0].ty.clone();
        let ir::Ty::Struct(nominal) = &owner else {
            return Ok(None);
        };
        if owner.owned_box_pointee().is_some()
            || self.explicit_field_stored(nominal, first)?
            || self.deref_target_hint(&owner)?.is_none()
        {
            return Ok(None);
        }
        Ok(Some(owner))
    }

    /// Project source selectors from a supplied source-owned carrier type.
    /// This performs only builtin reference/array and stored-field projections.
    /// No checked expression or checked local supplies a type here.
    pub(super) fn explicit_field_borrow_projection(
        &self,
        kind: ir::BorrowKind,
        mut ty: ir::Ty,
        fields: &[FieldSelector],
    ) -> Result<(Vec<ir::Projection>, ir::Ty), String> {
        if fields.len() > ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("place projection nesting exceeds limit128".into());
        }
        let mut projection = Vec::new();
        for field in fields {
            self.signatures.plan()?.budget.charge_work(1)?;
            match field {
                FieldSelector::Deref => {
                    let ir::Ty::Ref {
                        kind: layer,
                        referent,
                    } = &ty
                    else {
                        return Err("explicit projection dereference requires a reference".into());
                    };
                    if kind == ir::BorrowKind::Mut && *layer == ir::BorrowKind::Shared {
                        return Err(
                            "E0596: cannot borrow through a shared reference as mutable".into()
                        );
                    }
                    ty = referent.as_ref().clone();
                    self.explicit_field_projection_push(&mut projection, ir::Projection::Deref)?;
                }
                FieldSelector::ConstantIndex(index) => {
                    ty = self.index_type(&ty, *index)?;
                    self.explicit_field_projection_push(
                        &mut projection,
                        ir::Projection::ConstantIndex(*index),
                    )?;
                }
                _ => {
                    for _ in 0..ir::MAX_PATTERN_PROJECTION_DEPTH {
                        let ir::Ty::Ref {
                            kind: layer,
                            referent,
                        } = &ty
                        else {
                            break;
                        };
                        if kind == ir::BorrowKind::Mut && *layer == ir::BorrowKind::Shared {
                            return Err(
                                "E0596: cannot borrow through a shared reference as mutable".into(),
                            );
                        }
                        ty = referent.as_ref().clone();
                        self.explicit_field_projection_push(
                            &mut projection,
                            ir::Projection::Deref,
                        )?;
                    }
                    let (index, next) = self.field_type(&ty, field)?;
                    self.explicit_field_projection_push(
                        &mut projection,
                        ir::Projection::Field(index),
                    )?;
                    ty = next;
                }
            }
        }
        self.validate_borrow_referent(kind, &ty)?;
        Ok((projection, ty))
    }

    pub(super) fn explicit_field_borrow_hint(
        &self,
        kind: ir::BorrowKind,
        place: &Place,
    ) -> Result<Option<ir::Ty>, String> {
        let Some(owner) = self.explicit_field_borrow_owner(place)? else {
            return Ok(None);
        };
        let target = self
            .deref_target_hint(&owner)?
            .ok_or("Deref target disappeared")?;
        let carrier = ir::Ty::Ref {
            kind,
            referent: Arc::new(target),
        };
        let (_, referent) = self.explicit_field_borrow_projection(kind, carrier, &place.fields)?;
        Ok(Some(ir::Ty::Ref {
            kind,
            referent: Arc::new(referent),
        }))
    }

    pub(super) fn explicit_field_borrow(
        &mut self,
        source: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        let Expr::Borrow { kind, place } = source else {
            return Ok(None);
        };
        let Some(owner) = self.explicit_field_borrow_owner(place)? else {
            return Ok(None);
        };
        // The authored Borrow owns STEP1. This root Place is just the operand,
        // never an invented AST expression, pointer role or source occurrence.
        let root = Place {
            local: place.local,
            fields: Vec::new(),
        };
        let spec = if *kind == ir::BorrowKind::Mut {
            ("ops", "DerefMut", "deref_mut")
        } else {
            ("ops", "Deref", "deref")
        };
        let carrier = self.operator_call(
            spec,
            owner,
            None,
            operators::First::Place(*kind, &root),
            &[],
        )?;
        self.record_operator(
            deref_coercion::site(source as *const Expr as usize, 1),
            carrier,
        )?;
        let carrier_ty = self.exprs[carrier.0].ty.clone();
        let (projection, _) =
            self.explicit_field_borrow_projection(*kind, carrier_ty.clone(), &place.fields)?;
        let local = self.temporary("$explicit_deref_field", carrier_ty, false)?;
        self.register_borrow_temporary(local, false, None)?;
        let borrowed = self.borrow_expression(*kind, ir::Place { local, projection })?;
        let ty = self.exprs[borrowed.0].ty.clone();
        let result = self.for_expr(
            ty,
            ir::ExprKind::Block(ir::TypedBlock {
                statements: vec![ir::TypedStmt::Assign {
                    local,
                    value: carrier,
                }],
                tail: Some(borrowed),
            }),
        )?;
        Ok(Some(result))
    }
}
