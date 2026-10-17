//! Immutable compiler-owned canonical declarations. No source spelling can mint
//! an entry in this catalog. Logical rules and executable recipes are separate.
use super::*;
use crate::compiler::ir::{BinderId, BorrowKind, IntTy};

pub const CATALOG_VERSION: u32 = 2;
const BASE_COUNT: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecipeDescriptor {
    CloneInt(IntTy),
    CloneFloat(ir::FloatTy),
    CloneBool,
    CloneUnit,
    CloneNever,
    CloneTuple { arity: u16 },
    CloneArray { len: u16 },
    CloneSharedRef,
}
impl RecipeDescriptor {
    fn order(self) -> (u8, u16) {
        match self {
            Self::CloneInt(i) => (
                0,
                match i {
                    IntTy::I8 => 0,
                    IntTy::I16 => 1,
                    IntTy::I32 => 2,
                    IntTy::I64 => 3,
                    IntTy::Isize => 4,
                    IntTy::U8 => 5,
                    IntTy::U16 => 6,
                    IntTy::U32 => 7,
                    IntTy::U64 => 8,
                    IntTy::Usize => 9,
                },
            ),
            Self::CloneBool => (1, 0),
            Self::CloneUnit => (2, 0),
            Self::CloneNever => (3, 0),
            Self::CloneTuple { arity } => (4, arity),
            Self::CloneArray { len } => (5, len),
            Self::CloneSharedRef => (6, 0),
            Self::CloneFloat(float) => (
                7,
                match float {
                    ir::FloatTy::F32 => 0,
                    ir::FloatTy::F64 => 1,
                },
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CatalogItem {
    Sized,
    Clone,
    CloneItem,
    CloneFromItem,
    CloneFromDefault,
    Copy,
    Drop,
    DropItem,
    Recipe(RecipeDescriptor),
}
impl CatalogItem {
    fn base_index(self) -> Option<u32> {
        Some(match self {
            Self::Sized => 0,
            Self::Clone => 1,
            Self::CloneItem => 2,
            Self::CloneFromItem => 3,
            Self::CloneFromDefault => 4,
            Self::Copy => 5,
            Self::Drop => 6,
            Self::DropItem => 7,
            Self::Recipe(_) => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeclOrigin {
    Source,
    CoreCatalog { version: u32, item: CatalogItem },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallableProvider {
    Source { source_index: usize },
    Recipe { descriptor: RecipeDescriptor },
    UnavailableProvided { declaration: DeclId },
}
#[derive(Clone, Debug)]
pub struct CatalogDecl {
    pub declaration: DeclId,
    pub name: &'static str,
    pub info: SchemaDecl,
    pub schema: Option<GenericSchema>,
    pub origin: DeclOrigin,
    pub callable_provider: Option<CallableProvider>,
}
#[derive(Default)]
pub struct RecipeShapeInventory {
    entries: Vec<(RecipeDescriptor, super::super::ownership::RecipeAdmission)>,
}
impl RecipeShapeInventory {
    pub fn new() -> Self {
        Self::default()
    }
    pub(in crate::compiler) fn record(
        &mut self,
        descriptor: RecipeDescriptor,
        counts: &super::super::ownership::RecipePlanCounts,
        budget: &PlanBudget,
    ) -> Result<()> {
        let admission = super::super::ownership::preflight_recipe_counts(counts, budget)?;
        for (old, _) in &self.entries {
            charged(budget, budget.charge_work(1))?;
            if *old == descriptor {
                return Ok(());
            }
        }
        push(budget, &mut self.entries, (descriptor, admission))
    }
}

pub struct CoreCatalogBuilder {
    snapshot: SnapshotId,
    budget: PlanBudget,
    declarations: Vec<CatalogDecl>,
    headers: HeaderSet,
}
pub struct CoreCatalog {
    snapshot: SnapshotId,
    budget: PlanBudget,
    source_end: u32,
    declarations: Vec<CatalogDecl>,
    headers: HeaderSet,
    inventory: RecipeShapeInventory,
}
impl std::fmt::Debug for CoreCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreCatalog")
            .field("snapshot", &self.snapshot)
            .field("version", &CATALOG_VERSION)
            .finish_non_exhaustive()
    }
}
fn id(snapshot: SnapshotId, item: CatalogItem) -> DeclId {
    DeclId {
        snapshot,
        index: item.base_index().expect("private base catalog item"),
    }
}
fn scope(declaration: DeclId, arity: usize, budget: &PlanBudget) -> Result<Arc<ParamEnv>> {
    retain::<ParamEnv>(budget, 1)?;
    Ok(Arc::new(charged(
        budget,
        ParamEnv::new(BinderId { declaration }, arity),
    )?))
}
fn environment(scopes: &[Arc<ParamEnv>], budget: &PlanBudget) -> Result<Arc<GenericEnv>> {
    retain::<GenericEnv>(budget, 1)?;
    Ok(Arc::new(charged(budget, GenericEnv::new(scopes, budget))?))
}
fn reference(ty: Ty, kind: BorrowKind, budget: &PlanBudget) -> Result<Ty> {
    retain::<Ty>(budget, 1)?;
    Ok(Ty::Ref {
        kind,
        referent: Arc::new(ty),
    })
}
fn schema(
    env: Arc<GenericEnv>,
    params: Vec<Ty>,
    result: Ty,
    predicates: Vec<Predicate>,
    budget: &PlanBudget,
) -> Result<GenericSchema> {
    // Construct declared region positions, then use the same canonical IR
    // normalizer and rigid candidate derivation as source signatures.
    let mut inputs = Vec::new();
    let mut rows = Vec::new();
    let mut edges = Vec::new();
    let mut next = 0u16;
    for (param, ty) in params.iter().enumerate() {
        let layers = charged(budget, ir::reference_layers(ty))?;
        let mut prior = None;
        for depth in 0..layers {
            let region = ir::RegionParamId(next);
            next = next
                .checked_add(1)
                .ok_or_else(|| FactsError::resource(budget, "catalog region overflow"))?;
            push(
                budget,
                &mut inputs,
                ir::InputRegion {
                    input: ir::InputReference {
                        param,
                        position: ir::ReferencePosition { depth: depth as u8 },
                    },
                    region,
                },
            )?;
            if let Some(outer) = prior {
                push(budget, &mut edges, (region, outer))?;
            }
            prior = Some(region);
        }
    }
    for depth in 0..charged(budget, ir::reference_layers(&result))? {
        // Shared-reference recipe's result is its stored inner carrier, never
        // the transient outer handle. Other recipes have nonreference results.
        let input = inputs
            .iter()
            .find(|r| r.input.param == 0 && usize::from(r.input.position.depth) == depth + 1)
            .ok_or_else(|| {
                FactsError::invalid(budget, "catalog result has no declared carrier origin")
            })?;
        push(
            budget,
            &mut rows,
            ir::ResultReference {
                position: ir::ReferencePosition { depth: depth as u8 },
                region: input.region,
                returns: ir::ReturnContract::NoReference,
            },
        )?;
    }
    retain::<ir::RefContract>(budget, 1)?;
    let raw = ir::RefContract {
        region_count: next,
        input_regions: inputs.into(),
        results: rows.into(),
        outlives: edges.into(),
    };
    let normalized = charged(budget, ir::normalize_ref_contract(&raw, budget))?;
    let results = charged(
        budget,
        ir::derive_return_contracts(&params, &result, &normalized, budget),
    )?;
    retain::<ir::RefContract>(budget, 1)?;
    let contract = Arc::new(ir::RefContract {
        results,
        ..(*normalized).clone()
    });
    Ok(GenericSchema {
        env,
        signature: Signature {
            params,
            result,
            ref_contract: contract,
        },
        predicates: predicates.into(),
    })
}
impl CoreCatalogBuilder {
    pub fn new(snapshot: SnapshotId, budget: &PlanBudget) -> Result<Self> {
        retain::<Self>(budget, 1)?;
        let mut declarations = Vec::new();
        let mut traits = Vec::new();
        let mut items = Vec::new();
        let mut callables = Vec::new();
        for item in [
            CatalogItem::Sized,
            CatalogItem::Clone,
            CatalogItem::CloneItem,
            CatalogItem::CloneFromItem,
            CatalogItem::CloneFromDefault,
            CatalogItem::Copy,
            CatalogItem::Drop,
            CatalogItem::DropItem,
        ] {
            let declaration = id(snapshot, item);
            let is_trait = matches!(
                item,
                CatalogItem::Sized | CatalogItem::Clone | CatalogItem::Copy | CatalogItem::Drop
            );
            let owner = match item {
                CatalogItem::CloneItem
                | CatalogItem::CloneFromItem
                | CatalogItem::CloneFromDefault => Some(id(snapshot, CatalogItem::Clone)),
                CatalogItem::DropItem => Some(id(snapshot, CatalogItem::Drop)),
                _ => None,
            };
            let own = scope(declaration, usize::from(is_trait), budget)?;
            let mut scopes = Vec::new();
            if let Some(owner) = owner {
                push(budget, &mut scopes, scope(owner, 1, budget)?)?;
            }
            push(budget, &mut scopes, own.clone())?;
            let env = environment(&scopes, budget)?;
            let receiver = match item {
                CatalogItem::CloneItem => ReceiverMode::Borrowed {
                    kind: BorrowKind::Shared,
                },
                CatalogItem::CloneFromItem
                | CatalogItem::CloneFromDefault
                | CatalogItem::DropItem => ReceiverMode::Borrowed {
                    kind: BorrowKind::Mut,
                },
                _ => ReceiverMode::None,
            };
            let kind = if is_trait {
                DeclKind::Trait
            } else if item == CatalogItem::CloneFromDefault {
                DeclKind::AssociatedFunction
            } else {
                DeclKind::TraitItem
            };
            let name = match item {
                CatalogItem::Sized => "Sized",
                CatalogItem::Clone => "Clone",
                CatalogItem::CloneItem => "clone",
                CatalogItem::CloneFromItem => "clone_from",
                CatalogItem::CloneFromDefault => "clone_from_default",
                CatalogItem::Copy => "Copy",
                CatalogItem::Drop => "Drop",
                CatalogItem::DropItem => "drop",
                CatalogItem::Recipe(_) => unreachable!(),
            };
            let info = SchemaDecl {
                declaration,
                kind,
                own_env: own,
                env: env.clone(),
                owner,
                module: Arc::from([]),
                public: true,
                receiver,
                region_contract_mode: RegionContractMode::Positioned,
            };
            let mut raw = None;
            if is_trait {
                let self_param = env.ordered_params().next().ok_or_else(|| {
                    FactsError::invalid(budget, "catalog trait missing Self binder")
                })?;
                let mut required = Vec::new();
                match item {
                    CatalogItem::Clone => {
                        push(budget, &mut required, id(snapshot, CatalogItem::CloneItem))?;
                        push(
                            budget,
                            &mut required,
                            id(snapshot, CatalogItem::CloneFromItem),
                        )?;
                    }
                    CatalogItem::Drop => {
                        push(budget, &mut required, id(snapshot, CatalogItem::DropItem))?
                    }
                    _ => {}
                }
                let mut supers = Vec::new();
                let superitem = match item {
                    CatalogItem::Clone => Some(CatalogItem::Sized),
                    CatalogItem::Copy => Some(CatalogItem::Clone),
                    _ => None,
                };
                if let Some(s) = superitem {
                    push(
                        budget,
                        &mut supers,
                        TraitRef {
                            declaration: id(snapshot, s),
                            self_ty: Ty::Param(self_param),
                            args: Arc::from([]),
                        },
                    )?;
                }
                push(
                    budget,
                    &mut traits,
                    TraitHeader {
                        declaration,
                        self_param,
                        env: env.clone(),
                        items: required.into(),
                        supertraits: supers.into(),
                    },
                )?;
            } else {
                let owner = owner
                    .ok_or_else(|| FactsError::invalid(budget, "catalog item owner missing"))?;
                let self_ty = Ty::Param(ParamId {
                    owner: BinderId { declaration: owner },
                    index: 0,
                });
                let mut params = Vec::new();
                push(
                    budget,
                    &mut params,
                    reference(
                        self_ty.clone(),
                        if item == CatalogItem::CloneItem {
                            BorrowKind::Shared
                        } else {
                            BorrowKind::Mut
                        },
                        budget,
                    )?,
                )?;
                if matches!(
                    item,
                    CatalogItem::CloneFromItem | CatalogItem::CloneFromDefault
                ) {
                    push(
                        budget,
                        &mut params,
                        reference(self_ty.clone(), BorrowKind::Shared, budget)?,
                    )?;
                }
                let result = if item == CatalogItem::CloneItem {
                    self_ty
                } else {
                    Ty::Unit
                };
                let s = schema(env, params, result, Vec::new(), budget)?;
                if item == CatalogItem::CloneFromDefault {
                    push(
                        budget,
                        &mut callables,
                        CallableHeader {
                            declaration,
                            schema: clone_schema(&s, budget)?,
                        },
                    )?;
                } else {
                    push(
                        budget,
                        &mut items,
                        TraitItemHeader {
                            declaration,
                            trait_decl: owner,
                            receiver,
                            schema: clone_schema(&s, budget)?,
                            default: if item == CatalogItem::CloneFromItem {
                                Some(id(snapshot, CatalogItem::CloneFromDefault))
                            } else {
                                None
                            },
                        },
                    )?;
                }
                raw = Some(s);
            }
            push(
                budget,
                &mut declarations,
                CatalogDecl {
                    declaration,
                    name,
                    info,
                    schema: raw,
                    origin: DeclOrigin::CoreCatalog {
                        version: CATALOG_VERSION,
                        item,
                    },
                    callable_provider: if item == CatalogItem::CloneFromDefault {
                        Some(CallableProvider::UnavailableProvided { declaration })
                    } else {
                        None
                    },
                },
            )?;
        }
        Ok(Self {
            snapshot,
            budget: budget.clone(),
            declarations,
            headers: HeaderSet {
                traits: traits.into(),
                items: items.into(),
                impls: Arc::from([]),
                callables: callables.into(),
            },
        })
    }
    pub fn base_declarations(&self) -> &[CatalogDecl] {
        &self.declarations
    }
    pub fn source_start(&self) -> u32 {
        BASE_COUNT
    }
    pub fn finish(
        mut self,
        source_end: u32,
        mut inventory: RecipeShapeInventory,
        budget: &PlanBudget,
    ) -> Result<Arc<CoreCatalog>> {
        if !self.budget.same_authority(budget) || source_end < self.source_start() {
            return Err(FactsError::invalid(
                budget,
                "catalog layout or ledger mismatch",
            ));
        }
        charged(
            budget,
            budget.charge_work(
                inventory
                    .entries
                    .len()
                    .saturating_mul(inventory.entries.len().max(1)),
            ),
        )?;
        inventory.entries.sort_unstable_by_key(|(d, _)| d.order());
        let mut callables = Vec::new();
        for c in self.headers.callables.iter() {
            push(
                budget,
                &mut callables,
                CallableHeader {
                    declaration: c.declaration,
                    schema: clone_schema(&c.schema, budget)?,
                },
            )?;
        }
        let mut index = source_end;
        for (descriptor, admission) in &inventory.entries {
            if matches!(descriptor, RecipeDescriptor::CloneNever)
                || !matches!(
                    admission,
                    super::super::ownership::RecipeAdmission::Available(_)
                )
            {
                continue;
            }
            let declaration = DeclId {
                snapshot: self.snapshot,
                index,
            };
            index = index.checked_add(1).ok_or_else(|| {
                FactsError::resource(budget, "catalog declaration index overflow")
            })?;
            let s = recipe_schema(
                declaration,
                *descriptor,
                self.id(CatalogItem::Sized),
                self.id(CatalogItem::Clone),
                budget,
            )?;
            let own = s
                .env
                .scopes()
                .last()
                .ok_or_else(|| FactsError::invalid(budget, "recipe scope missing"))?
                .clone();
            let info = SchemaDecl {
                declaration,
                kind: DeclKind::Function,
                own_env: own,
                env: s.env.clone(),
                owner: None,
                module: Arc::from([]),
                public: true,
                receiver: ReceiverMode::Borrowed {
                    kind: BorrowKind::Shared,
                },
                region_contract_mode: RegionContractMode::Positioned,
            };
            push(
                budget,
                &mut callables,
                CallableHeader {
                    declaration,
                    schema: clone_schema(&s, budget)?,
                },
            )?;
            push(
                budget,
                &mut self.declarations,
                CatalogDecl {
                    declaration,
                    name: "clone",
                    info,
                    schema: Some(s),
                    origin: DeclOrigin::CoreCatalog {
                        version: CATALOG_VERSION,
                        item: CatalogItem::Recipe(*descriptor),
                    },
                    callable_provider: Some(CallableProvider::Recipe {
                        descriptor: *descriptor,
                    }),
                },
            )?;
        }
        self.headers.callables = callables.into();
        retain::<CoreCatalog>(budget, 1)?;
        Ok(Arc::new(CoreCatalog {
            snapshot: self.snapshot,
            budget: self.budget,
            source_end,
            declarations: self.declarations,
            headers: self.headers,
            inventory,
        }))
    }
    fn id(&self, item: CatalogItem) -> DeclId {
        id(self.snapshot, item)
    }
}
fn recipe_schema(
    declaration: DeclId,
    descriptor: RecipeDescriptor,
    sized: DeclId,
    clone: DeclId,
    budget: &PlanBudget,
) -> Result<GenericSchema> {
    let arity = match descriptor {
        RecipeDescriptor::CloneTuple { arity } => usize::from(arity),
        RecipeDescriptor::CloneArray { .. } | RecipeDescriptor::CloneSharedRef => 1,
        _ => 0,
    };
    let env = environment(&[scope(declaration, arity, budget)?], budget)?;
    let mut types = Vec::new();
    let mut predicates = Vec::new();
    for p in env.ordered_params() {
        push(budget, &mut types, Ty::Param(p))?;
        push(
            budget,
            &mut predicates,
            Predicate::Implements(TraitRef {
                declaration: sized,
                self_ty: Ty::Param(p),
                args: Arc::from([]),
            }),
        )?;
        if !matches!(descriptor, RecipeDescriptor::CloneSharedRef) {
            push(
                budget,
                &mut predicates,
                Predicate::Implements(TraitRef {
                    declaration: clone,
                    self_ty: Ty::Param(p),
                    args: Arc::from([]),
                }),
            )?;
        }
    }
    let value = match descriptor {
        RecipeDescriptor::CloneInt(i) => Ty::Int(i),
        RecipeDescriptor::CloneFloat(float) => Ty::Float(float),
        RecipeDescriptor::CloneBool => Ty::Bool,
        RecipeDescriptor::CloneUnit => Ty::Unit,
        RecipeDescriptor::CloneNever => {
            return Err(FactsError::unsupported(
                budget,
                "CloneNever has no executable body",
            ));
        }
        RecipeDescriptor::CloneTuple { arity: 0 } => Ty::Unit,
        RecipeDescriptor::CloneTuple { .. } => Ty::Tuple(types.into()),
        RecipeDescriptor::CloneArray { len } => {
            retain::<Ty>(budget, 1)?;
            Ty::Array {
                element: Arc::new(types.remove(0)),
                len: usize::from(len),
            }
        }
        RecipeDescriptor::CloneSharedRef => reference(types.remove(0), BorrowKind::Shared, budget)?,
    };
    let mut params = Vec::new();
    push(
        budget,
        &mut params,
        reference(value.clone(), BorrowKind::Shared, budget)?,
    )?;
    schema(env, params, value, predicates, budget)
}
impl CoreCatalog {
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn version(&self) -> u32 {
        CATALOG_VERSION
    }
    pub fn budget(&self) -> &PlanBudget {
        &self.budget
    }
    pub fn source_start(&self) -> u32 {
        BASE_COUNT
    }
    pub fn source_end(&self) -> u32 {
        self.source_end
    }
    pub fn id(&self, item: CatalogItem) -> DeclId {
        id(self.snapshot, item)
    }
    pub fn declarations(&self) -> &[CatalogDecl] {
        &self.declarations
    }
    pub fn headers(&self) -> &HeaderSet {
        &self.headers
    }
    pub fn declaration(&self, id: DeclId) -> Option<&CatalogDecl> {
        if id.snapshot != self.snapshot {
            return None;
        }
        // Catalog allocation has two disjoint ranges, but range is never the
        // authority: exact retained declaration identity still must match.
        if id.index < BASE_COUNT {
            self.declarations
                .get(id.index as usize)
                .filter(|d| d.declaration == id)
        } else {
            let i = usize::try_from(id.index.checked_sub(self.source_end)?)
                .ok()?
                .checked_add(BASE_COUNT as usize)?;
            self.declarations.get(i).filter(|d| d.declaration == id)
        }
    }
    pub fn recipe(&self, descriptor: RecipeDescriptor) -> Result<&CatalogDecl> {
        if descriptor == RecipeDescriptor::CloneNever {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: CloneNever has no executable body",
            ));
        }
        for d in &self.declarations {
            charged(&self.budget, self.budget.charge_work(1))?;
            if d.callable_provider == Some(CallableProvider::Recipe { descriptor }) {
                return Ok(d);
            }
        }
        for (d, admission) in &self.inventory.entries {
            charged(&self.budget, self.budget.charge_work(1))?;
            if *d == descriptor {
                return Err(match admission {
                    super::super::ownership::RecipeAdmission::IntrinsicLimit(reason) => {
                        FactsError::resource(&self.budget, reason.message())
                    }
                    super::super::ownership::RecipeAdmission::Available(_) => FactsError::invalid(
                        &self.budget,
                        "admitted recipe has no canonical provider",
                    ),
                });
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "Clone recipe shape was not inventoried",
        ))
    }
    pub fn origin(&self, declaration: DeclId) -> Result<DeclOrigin> {
        if declaration.snapshot != self.snapshot {
            return Err(FactsError::invalid(
                &self.budget,
                "foreign catalog origin query",
            ));
        }
        if let Some(d) = self.declaration(declaration) {
            return Ok(d.origin);
        }
        Err(FactsError::invalid(
            &self.budget,
            "declaration is not an authoritative catalog record",
        ))
    }
}
/// The shared checked layout kernel is also used by synthetic fixture callers.
/// It changes the constructed source identity explicitly, never remaps a supplied
/// identity inside a world or facts factory.
#[cfg(test)]
pub(super) fn fixture_source_decl(
    snapshot: SnapshotId,
    ordinal: u32,
    budget: &PlanBudget,
) -> Result<DeclId> {
    charged(budget, budget.charge_work(1))?;
    let index = BASE_COUNT
        .checked_add(ordinal)
        .ok_or_else(|| FactsError::resource(budget, "fixture source declaration index overflow"))?;
    Ok(DeclId { snapshot, index })
}
