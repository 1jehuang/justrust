//! Safe reader-owned nominal shells and explicit one-level field projections.
//! Published component facts contain symbols only. Every executable nominal
//! child retains a successful component read, with no back edge or weak owner.
use super::{
    Error, FieldCoordinate, OutputIdentity, ReadNominal, RealizationKey, component::ReadComponent,
    coordinates::Term, ir, producer,
};
use std::sync::Arc;

fn copied_name(name: &str, b: &ir::PlanBudget) -> Result<String, Error> {
    b.retain_bytes(name.len()).map_err(Error::new)?;
    let mut copy = String::new();
    copy.try_reserve_exact(name.len())
        .map_err(|_| Error::new("nominal shell name allocation failed"))?;
    copy.push_str(name);
    Ok(copy)
}

/// Sealed immutable projection stored in ir::NominalFields. Clone retains its
/// actual typed component authority, so escaped fields remain independently safe.
#[derive(Clone, Debug)]
pub(in crate::compiler) struct SymbolicFields {
    read: ReadComponent,
    member: u32,
    variant: Option<usize>,
    len: usize,
}
impl SymbolicFields {
    pub(in crate::compiler) fn len(&self) -> usize {
        self.len
    }
    pub(in crate::compiler) fn read(
        &self,
        b: &ir::PlanBudget,
    ) -> Result<Arc<[ir::FieldDef]>, String> {
        self.read
            .fields(self.member, self.variant, b)
            .map_err(|e| e.to_string())
    }
}

#[derive(Clone)]
pub(in crate::compiler) struct RootedNominal {
    read: ReadComponent,
    member: u32,
    /// Exact reader-owned raw shell allocation, without its own ReadNominal.
    /// Its fields retain only the component fact, not this shell or capability.
    definition: Arc<ir::Ty>,
}
impl RootedNominal {
    pub(super) fn key(&self) -> &RealizationKey {
        self.read.member_key(self.member).expect("sealed member")
    }
    pub(super) fn identity(&self) -> OutputIdentity {
        self.read.identity()
    }
    pub(super) fn metadata_height(&self) -> usize {
        self.read.metadata_height(self.member)
    }
    pub(super) fn reference_type_summary(
        &self,
        budget: &ir::PlanBudget,
        depth: usize,
        visits: &mut usize,
    ) -> Result<(bool, usize), Error> {
        self.read
            .reference_type_summary(self.member, budget, depth, visits)
    }
    pub(super) fn slice_boundary_height(
        &self,
        allow_references: bool,
        allow_universal_projections: bool,
        budget: &ir::PlanBudget,
        depth: usize,
        work: &mut usize,
    ) -> Result<usize, Error> {
        self.read.slice_boundary_height(
            self.member,
            allow_references,
            allow_universal_projections,
            budget,
            depth,
            work,
        )
    }
    pub(super) fn drop_presence(
        &self,
        budget: &ir::PlanBudget,
        depth: usize,
        event: &mut impl FnMut(&ir::Ty, bool, usize) -> Result<(bool, bool), Error>,
    ) -> Result<(bool, bool), Error> {
        self.read.drop_presence(self.member, budget, depth, event)
    }
    pub(super) fn ownership_preflight(
        &self,
        budget: &ir::PlanBudget,
        depth: usize,
        event: &mut impl FnMut(usize, Option<(&ir::Ty, usize)>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.read
            .ownership_preflight(self.member, budget, depth, event)
    }
    pub(super) fn pointer_kind(&self) -> Option<ir::OwnedPointerKind> {
        self.read.pointer_kind(self.member)
    }
    pub(super) fn source_application(&self) -> ir::NominalAppId {
        match self.read.source(self.member).expect("sealed member") {
            ir::Ty::Struct(n) => n.id(),
            ir::Ty::Enum(n) => n.id(),
            _ => unreachable!("validated nominal"),
        }
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        self.member == other.member && self.read.same(&other.read)
    }
    pub(super) fn check(&self, ty: &ir::Ty) -> Result<(), Error> {
        let source = self.read.source(self.member)?;
        let expected_index =
            ir::NominalIndex::GraphComponentMember(self.read.identity(), self.member);
        let expected_source = match source {
            ir::Ty::Struct(n) => (n.id().snapshot, n.def().source_decl),
            ir::Ty::Enum(n) => (n.id().snapshot, n.def().source_decl),
            _ => return Err(Error::new("component member source changed kind")),
        };
        let same = match (ty, self.definition.as_ref()) {
            (ir::Ty::Struct(a), ir::Ty::Struct(b)) => {
                std::ptr::eq(a.def(), b.def())
                    && a.id() == b.id()
                    && a.id().index == expected_index
                    && (a.id().snapshot, a.def().source_decl) == expected_source
            }
            (ir::Ty::Enum(a), ir::Ty::Enum(b)) => {
                std::ptr::eq(a.def(), b.def())
                    && a.id() == b.id()
                    && a.id().index == expected_index
                    && (a.id().snapshot, a.def().source_decl) == expected_source
            }
            _ => false,
        };
        if !same {
            return Err(Error::new(
                "component nominal definition differs from exact rooted read allocation",
            ));
        }
        Ok(())
    }
    pub(super) fn definition(&self) -> Result<ir::Ty, Error> {
        let capability = ReadNominal::Component(self.clone());
        Ok(match self.definition.as_ref() {
            ir::Ty::Struct(n) => ir::Ty::Struct(n.clone().with_realization(capability)),
            ir::Ty::Enum(n) => ir::Ty::Enum(n.clone().with_realization(capability)),
            _ => return Err(Error::new("component root is not nominal")),
        })
    }
    pub(super) fn replay(
        &self,
        ty: &ir::Ty,
        world: &super::traits::ValidatedTraitWorld,
    ) -> Result<(), Error> {
        self.check(ty)?;
        self.read.replay(world)
    }
    pub(super) fn replay_at(
        &self,
        ty: &ir::Ty,
        world: &super::traits::ValidatedTraitWorld,
        depth: usize,
    ) -> Result<(), Error> {
        self.check(ty)?;
        self.read.replay_at(world, depth)
    }
}

impl ReadComponent {
    pub(super) fn materialize(
        &self,
        member: u32,
        b: &ir::PlanBudget,
    ) -> Result<RootedNominal, Error> {
        self.shell(member, b, 0)
    }

    pub(super) fn resolve(
        &self,
        term: &Term,
        b: &ir::PlanBudget,
        depth: usize,
    ) -> Result<ir::Ty, Error> {
        b.check_depth(depth).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        b.retain_bytes(std::mem::size_of::<ir::Ty>())
            .map_err(Error::new)?;
        let child = |t: &Term| self.resolve(t, b, depth + 1);
        Ok(match term {
            Term::Int(t) => ir::Ty::Int(*t),
            Term::Float(t) => ir::Ty::Float(*t),
            Term::Bool => ir::Ty::Bool,
            Term::Unit => ir::Ty::Unit,
            Term::Never => ir::Ty::Never,
            Term::Char => ir::Ty::Char,
            Term::Ref(kind, t) => ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(child(t)?),
            },
            Term::RawPtr(kind, t) => ir::Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(child(t)?),
            },
            Term::Array(t, len) => ir::Ty::Array {
                element: Arc::new(child(t)?),
                len: *len,
            },
            Term::PhantomData(..) => {
                return Err(Error::new(
                    "unsupported: recursive PhantomData materialization requires sealed catalog construction authority",
                ));
            }
            Term::FnPointer(inputs, output) => {
                let mut types = ir::FnPointerTy::storage(inputs.len(), b).map_err(Error::new)?;
                for t in inputs {
                    types.push(child(t)?);
                }
                ir::Ty::FnPointer(
                    ir::FnPointerTy::new(types, child(output)?, b).map_err(Error::new)?,
                )
            }
            Term::Tuple(types) => {
                let mut values = super::topology::storage(types.len(), b)?;
                for t in types {
                    values.push(child(t)?);
                }
                ir::Ty::Tuple(values.into())
            }
            Term::Nominal(declaration, actuals) => {
                let key = producer::key(self.world_word(), *declaration, actuals, b)?;
                if let Some(member) = self.find_member(&key) {
                    self.shell(member, b, depth + 1)?.definition()?
                } else {
                    self.external(&key)
                        .ok_or_else(|| {
                            Error::new("symbolic field omitted successful external read")
                        })?
                        .definition()?
                }
            }
            Term::Str | Term::Slice(_) | Term::Dyn(..) => {
                return Err(Error::new(
                    "unsupported: recursive component unsized field materialization",
                ));
            }
        })
    }

    /// Build just the header and finite actual syntax. Nominal fields are
    /// immutable symbolic projections, not recursively expanded definitions.
    fn shell(&self, member: u32, b: &ir::PlanBudget, depth: usize) -> Result<RootedNominal, Error> {
        b.check_depth(depth).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        let source = self.source(member)?;
        let terms = self.actuals(member)?;
        let mut actuals = super::topology::storage(terms.len(), b)?;
        for term in terms {
            actuals.push(self.resolve(term, b, depth + 1)?);
        }
        let actuals: Arc<[ir::Ty]> = actuals.into();
        let id = |original: ir::NominalAppId| ir::NominalAppId {
            snapshot: original.snapshot,
            index: ir::NominalIndex::GraphComponentMember(self.identity(), member),
        };
        let fields = |variant, len| {
            ir::NominalFields::symbolic(SymbolicFields {
                read: self.clone(),
                member,
                variant,
                len,
            })
        };
        let definition = match source {
            ir::Ty::Struct(n) => {
                b.retain_bytes(std::mem::size_of::<ir::StructDef>())
                    .map_err(Error::new)?;
                let source = n.def();
                ir::Ty::Struct(
                    ir::StructTy::new(ir::StructDef {
                        id: id(source.id),
                        source_decl: source.source_decl,
                        type_args: actuals,
                        name: copied_name(&source.name, b)?,
                        module: source.module,
                        visibility: source.visibility.clone(),
                        fields: fields(None, source.fields.len()),
                    })
                    .with_native_sequence_binding(n.native_sequence_binding().cloned()),
                )
            }
            ir::Ty::Enum(n) => {
                b.retain_bytes(std::mem::size_of::<ir::EnumDef>())
                    .map_err(Error::new)?;
                let source = n.def();
                let mut variants = super::topology::storage(source.variants.len(), b)?;
                for (index, original) in source.variants.iter().enumerate() {
                    b.charge_work(1).map_err(Error::new)?;
                    variants.push(ir::VariantDef {
                        name: copied_name(&original.name, b)?,
                        shape: original.shape,
                        fields: fields(Some(index), original.fields.len()),
                    });
                }
                ir::Ty::Enum(ir::EnumTy::new(ir::EnumDef {
                    id: id(source.id),
                    source_decl: source.source_decl,
                    type_args: actuals,
                    name: copied_name(&source.name, b)?,
                    module: source.module,
                    visibility: source.visibility.clone(),
                    variants: variants.into(),
                }))
            }
            _ => return Err(Error::new("component source shell is not nominal")),
        };
        b.retain_bytes(std::mem::size_of::<ir::Ty>())
            .map_err(Error::new)?;
        Ok(RootedNominal {
            read: self.clone(),
            member,
            definition: Arc::new(definition),
        })
    }

    fn fields(
        &self,
        member: u32,
        variant: Option<usize>,
        b: &ir::PlanBudget,
    ) -> Result<Arc<[ir::FieldDef]>, Error> {
        let original = match (self.source(member)?, variant) {
            (ir::Ty::Struct(n), None) => n.def().fields.read(b).map_err(Error::new)?,
            (ir::Ty::Enum(n), Some(variant)) => n
                .def()
                .variants
                .get(variant)
                .ok_or_else(|| Error::new("symbolic variant ordinal out of bounds"))?
                .fields
                .read(b)
                .map_err(Error::new)?,
            _ => return Err(Error::new("symbolic field projection kind mismatch")),
        };
        let mut fields = super::topology::storage(original.len(), b)?;
        for (field, source) in original.iter().enumerate() {
            b.charge_work(1).map_err(Error::new)?;
            let coordinate = FieldCoordinate { variant, field };
            let term = self.field_term(member, coordinate)?;
            fields.push(ir::FieldDef {
                name: copied_name(&source.name, b)?,
                ty: self.resolve(term, b, 0)?,
                visibility: source.visibility.clone(),
                lifetime_bound: source.lifetime_bound,
            });
        }
        Ok(fields.into())
    }
}
