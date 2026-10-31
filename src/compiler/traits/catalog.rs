//! Immutable compiler-owned canonical declarations. No source spelling can mint
//! an entry in this catalog. Logical rules and executable recipes are separate.
use super::*;
use crate::compiler::ir::{BinderId, BorrowKind, IntTy};
use crate::compiler::module_identity::CatalogModule;
use crate::compiler::semantic_name::BuiltinAtom;

pub const CATALOG_VERSION: u32 = 10;

const BASE_COUNT: u32 = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecipeDescriptor {
    CloneInt(IntTy),
    CloneFloat(ir::FloatTy),
    CloneBool,
    CloneChar,
    CloneUnit,
    CloneNever,
    CloneTuple { arity: u16 },
    CloneArray { len: u16 },
    CloneSharedRef,
    ClonePhantomData,
    CloneFromPhantomData,
}
impl RecipeDescriptor {
    fn order(self) -> (u8, u16) {
        match self {
            Self::CloneInt(i) => match i {
                IntTy::I8 => (0, 0),
                IntTy::I16 => (0, 1),
                IntTy::I32 => (0, 2),
                IntTy::I64 => (0, 3),
                IntTy::Isize => (0, 4),
                IntTy::U8 => (0, 5),
                IntTy::U16 => (0, 6),
                IntTy::U32 => (0, 7),
                IntTy::U64 => (0, 8),
                IntTy::Usize => (0, 9),
                // Append after every existing family, preserving old catalog IDs.
                IntTy::I128 => (8, 0),
                IntTy::U128 => (8, 1),
            },
            Self::CloneBool => (1, 0),
            Self::CloneChar => (9, 0),
            Self::CloneUnit => (2, 0),
            Self::CloneNever => (3, 0),
            Self::CloneTuple { arity } => (4, arity),
            Self::CloneArray { len } => (5, len),
            Self::CloneSharedRef => (6, 0),
            Self::ClonePhantomData => (10, 0),
            Self::CloneFromPhantomData => (11, 0),
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
    PhantomData,
    ManuallyDrop,
    ManuallyDropNew,
    ManuallyDropIntoInner,
    ManuallyDropDrop,
    Recipe(RecipeDescriptor),
}
impl CatalogItem {
    pub fn module(self) -> CatalogModule {
        match self {
            Self::Sized | Self::Copy | Self::PhantomData => CatalogModule::Marker,
            Self::ManuallyDrop
            | Self::ManuallyDropNew
            | Self::ManuallyDropIntoInner
            | Self::ManuallyDropDrop => CatalogModule::Mem,
            Self::Clone
            | Self::CloneItem
            | Self::CloneFromItem
            | Self::CloneFromDefault
            | Self::Recipe(_) => CatalogModule::Clone,
            Self::Drop | Self::DropItem => CatalogModule::Ops,
        }
    }
    pub fn binding_atom(self) -> Option<BuiltinAtom> {
        match self {
            Self::Sized => Some(BuiltinAtom::Sized),
            Self::Clone => Some(BuiltinAtom::Clone),
            Self::Copy => Some(BuiltinAtom::Copy),
            Self::Drop => Some(BuiltinAtom::Drop),
            Self::PhantomData => Some(BuiltinAtom::PhantomData),
            Self::ManuallyDrop => Some(BuiltinAtom::ManuallyDrop),
            Self::ManuallyDropNew => Some(BuiltinAtom::ManuallyDropNew),
            Self::ManuallyDropIntoInner => Some(BuiltinAtom::ManuallyDropIntoInner),
            Self::ManuallyDropDrop => Some(BuiltinAtom::ManuallyDropDrop),
            _ => None,
        }
    }
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
            Self::PhantomData => 8,
            Self::ManuallyDrop => 9,
            Self::ManuallyDropNew => 10,
            Self::ManuallyDropIntoInner => 11,
            Self::ManuallyDropDrop => 12,
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
    modules: Arc<ModuleArena>,
    budget: PlanBudget,
    declarations: Vec<CatalogDecl>,
    /// Explicit bounded catalog declaration predicates, never inferred from spelling.
    declared_bounds: Arc<[Predicate]>,
    headers: HeaderSet,
}
pub struct CoreCatalog {
    snapshot: SnapshotId,
    modules: Arc<ModuleArena>,
    budget: PlanBudget,
    source_end: u32,
    declarations: Vec<CatalogDecl>,
    /// Explicit bounded catalog declaration predicates, never inferred from spelling.
    declared_bounds: Arc<[Predicate]>,
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
        index: item.base_index().expect("private base catalog item").into(),
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
        static_regions: 0,
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
    pub fn new(
        snapshot: SnapshotId,
        modules: Arc<ModuleArena>,
        budget: &PlanBudget,
    ) -> Result<Self> {
        retain::<Self>(budget, 1)?;
        checked_module(budget, modules.check(modules.core_root()))?;
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
            CatalogItem::PhantomData,
            CatalogItem::ManuallyDrop,
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
            let own = scope(
                declaration,
                usize::from(
                    is_trait
                        || matches!(item, CatalogItem::PhantomData | CatalogItem::ManuallyDrop),
                ),
                budget,
            )?;
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
            let kind = if matches!(item, CatalogItem::PhantomData | CatalogItem::ManuallyDrop) {
                DeclKind::Struct
            } else if is_trait {
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
                CatalogItem::PhantomData => "PhantomData",
                CatalogItem::ManuallyDrop => "ManuallyDrop",
                CatalogItem::ManuallyDropNew
                | CatalogItem::ManuallyDropIntoInner
                | CatalogItem::ManuallyDropDrop => unreachable!(),
                CatalogItem::Recipe(_) => unreachable!(),
            };
            let info = SchemaDecl {
                declaration,
                kind,
                own_env: own,
                env: env.clone(),
                owner,
                module: modules.catalog_module(item.module()),
                visibility: crate::compiler::ir::Visibility::Public,
                receiver,
                region_contract_mode: RegionContractMode::Positioned,
            };
            checked_module(budget, info.visibility.validate_for(&modules, info.module))?;
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
                        associated_types: Arc::from([]),
                        supertraits: supers.into(),
                    },
                )?;
            } else if !matches!(item, CatalogItem::PhantomData | CatalogItem::ManuallyDrop) {
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
        let mut declared_bounds = Vec::new();
        retain::<Ty>(budget, 0)?;
        push(
            budget,
            &mut declared_bounds,
            Predicate::Implements(TraitRef {
                declaration: id(snapshot, CatalogItem::Sized),
                self_ty: Ty::Param(ParamId {
                    owner: BinderId {
                        declaration: id(snapshot, CatalogItem::ManuallyDrop),
                    },
                    index: 0,
                }),
                args: Arc::from([]),
            }),
        )?;
        retain::<Predicate>(budget, declared_bounds.len())?;
        let mut builder = Self {
            snapshot,
            modules,
            budget: budget.clone(),
            declarations,
            declared_bounds: declared_bounds.into(),
            headers: HeaderSet {
                constants: Arc::from([]),
                associated_types: Arc::from([]),
                traits: traits.into(),
                items: items.into(),
                impls: Arc::from([]),
                callables: callables.into(),
            },
        };
        builder.add_manually_drop_operations(budget)?;
        Ok(builder)
    }
    fn add_manually_drop_operations(&mut self, budget: &PlanBudget) -> Result<()> {
        let factory = charged(budget, ir::ManuallyDropFactory::from_builder(self, budget))?;
        let owner = factory.declaration();
        let mut callables = Vec::new();
        for header in self.headers.callables.iter() {
            push(
                budget,
                &mut callables,
                CallableHeader {
                    declaration: header.declaration,
                    schema: clone_schema(&header.schema, budget)?,
                },
            )?;
        }
        for (item, name) in [
            (CatalogItem::ManuallyDropNew, "new"),
            (CatalogItem::ManuallyDropIntoInner, "into_inner"),
            (CatalogItem::ManuallyDropDrop, "drop"),
        ] {
            let declaration = id(self.snapshot, item);
            let own = scope(declaration, 0, budget)?;
            let mut scopes = Vec::new();
            push(budget, &mut scopes, scope(owner, 1, budget)?)?;
            push(budget, &mut scopes, own.clone())?;
            let env = environment(&scopes, budget)?;
            let argument = Ty::Param(ParamId {
                owner: BinderId { declaration: owner },
                index: 0,
            });
            let wrapper = Ty::ManuallyDrop(charged(
                budget,
                factory.with_argument(argument.clone(), budget),
            )?);
            let mut params = Vec::new();
            let (input, result) = match item {
                CatalogItem::ManuallyDropNew => (argument.clone(), wrapper),
                CatalogItem::ManuallyDropIntoInner => (wrapper, argument.clone()),
                CatalogItem::ManuallyDropDrop => {
                    (reference(wrapper, BorrowKind::Mut, budget)?, Ty::Unit)
                }
                _ => unreachable!(),
            };
            push(budget, &mut params, input)?;
            let mut predicates = Vec::new();
            retain::<Ty>(budget, 0)?;
            push(
                budget,
                &mut predicates,
                Predicate::Implements(TraitRef {
                    declaration: id(self.snapshot, CatalogItem::Sized),
                    self_ty: argument,
                    args: Arc::from([]),
                }),
            )?;
            retain::<Predicate>(budget, predicates.len())?;
            retain::<ir::InputRegion>(budget, usize::from(item == CatalogItem::ManuallyDropDrop))?;
            retain::<ir::ResultReference>(budget, 0)?;
            retain::<(ir::RegionParamId, ir::RegionParamId)>(budget, 0)?;
            let signature = schema(env.clone(), params, result, predicates, budget)?;
            push(
                budget,
                &mut callables,
                CallableHeader {
                    declaration,
                    schema: clone_schema(&signature, budget)?,
                },
            )?;
            push(
                budget,
                &mut self.declarations,
                CatalogDecl {
                    declaration,
                    name,
                    info: SchemaDecl {
                        declaration,
                        kind: DeclKind::AssociatedFunction,
                        own_env: own,
                        env,
                        owner: Some(owner),
                        module: self.modules.catalog_module(CatalogModule::Mem),
                        visibility: ir::Visibility::Public,
                        receiver: ReceiverMode::None,
                        region_contract_mode: RegionContractMode::Positioned,
                    },
                    schema: Some(signature),
                    origin: DeclOrigin::CoreCatalog {
                        version: CATALOG_VERSION,
                        item,
                    },
                    callable_provider: None,
                },
            )?;
        }
        retain::<CallableHeader>(budget, callables.len())?;
        self.headers.callables = callables.into();
        Ok(())
    }
    pub fn base_declarations(&self) -> &[CatalogDecl] {
        &self.declarations
    }
    pub fn modules(&self) -> &Arc<ModuleArena> {
        &self.modules
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
        checked_module(budget, self.modules.check(self.modules.core_root()))?;
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
                index: index.into(),
            };
            index = index.checked_add(1).ok_or_else(|| {
                FactsError::resource(budget, "catalog declaration index overflow")
            })?;
            let s = recipe_schema(
                declaration,
                *descriptor,
                self.id(CatalogItem::Sized),
                self.id(CatalogItem::Clone),
                if matches!(
                    descriptor,
                    RecipeDescriptor::ClonePhantomData | RecipeDescriptor::CloneFromPhantomData
                ) {
                    Some(charged(
                        budget,
                        ir::PhantomDataFactory::from_builder(&self, budget),
                    )?)
                } else {
                    None
                },
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
                module: self.modules.catalog_module(CatalogModule::Clone),
                visibility: crate::compiler::ir::Visibility::Public,
                receiver: ReceiverMode::Borrowed {
                    kind: if *descriptor == RecipeDescriptor::CloneFromPhantomData {
                        BorrowKind::Mut
                    } else {
                        BorrowKind::Shared
                    },
                },
                region_contract_mode: RegionContractMode::Positioned,
            };
            checked_module(
                budget,
                info.visibility.validate_for(&self.modules, info.module),
            )?;
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
            modules: self.modules,
            budget: self.budget,
            source_end,
            declarations: self.declarations,
            declared_bounds: self.declared_bounds,
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
    marker_factory: Option<ir::PhantomDataFactory>,
    budget: &PlanBudget,
) -> Result<GenericSchema> {
    let arity = match descriptor {
        RecipeDescriptor::CloneTuple { arity } => usize::from(arity),
        RecipeDescriptor::CloneArray { .. }
        | RecipeDescriptor::CloneSharedRef
        | RecipeDescriptor::ClonePhantomData
        | RecipeDescriptor::CloneFromPhantomData => 1,
        _ => 0,
    };
    let own = if matches!(descriptor, RecipeDescriptor::CloneSharedRef) {
        retain::<ParamEnv>(budget, 1)?;
        Arc::new(charged(
            budget,
            ParamEnv::with_relaxed_sized(BinderId { declaration }, arity, 1),
        )?)
    } else {
        scope(declaration, arity, budget)?
    };
    let env = environment(&[own], budget)?;
    let mut types = Vec::new();
    let mut predicates = Vec::new();
    for p in env.ordered_params() {
        push(budget, &mut types, Ty::Param(p))?;
        // `impl<T: ?Sized> Clone for &T`: the referent need not be Sized.
        if matches!(descriptor, RecipeDescriptor::CloneSharedRef) {
            continue;
        }
        push(
            budget,
            &mut predicates,
            Predicate::Implements(TraitRef {
                declaration: sized,
                self_ty: Ty::Param(p),
                args: Arc::from([]),
            }),
        )?;
        if !matches!(
            descriptor,
            RecipeDescriptor::ClonePhantomData | RecipeDescriptor::CloneFromPhantomData
        ) {
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
        RecipeDescriptor::CloneChar => Ty::Char,
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
        RecipeDescriptor::ClonePhantomData | RecipeDescriptor::CloneFromPhantomData => {
            Ty::PhantomData(charged(
                budget,
                marker_factory
                    .ok_or_else(|| {
                        FactsError::invalid(budget, "marker clone recipe lost sealed factory")
                    })?
                    .with_argument(types.remove(0), budget),
            )?)
        }
    };
    let mut params = Vec::new();
    if descriptor == RecipeDescriptor::CloneFromPhantomData {
        push(
            budget,
            &mut params,
            reference(value.clone(), BorrowKind::Mut, budget)?,
        )?;
        push(
            budget,
            &mut params,
            reference(value, BorrowKind::Shared, budget)?,
        )?;
        schema(env, params, Ty::Unit, predicates, budget)
    } else {
        push(
            budget,
            &mut params,
            reference(value.clone(), BorrowKind::Shared, budget)?,
        )?;
        schema(env, params, value, predicates, budget)
    }
}
impl CoreCatalog {
    pub fn modules(&self) -> &Arc<ModuleArena> {
        &self.modules
    }
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
        if id.index < u64::from(BASE_COUNT) {
            self.declarations
                .get(id.index as usize)
                .filter(|d| d.declaration == id)
        } else {
            let i = usize::try_from(id.index.checked_sub(u64::from(self.source_end))?)
                .ok()?
                .checked_add(BASE_COUNT as usize)?;
            self.declarations.get(i).filter(|d| d.declaration == id)
        }
    }
    /// Raw header construction may read only explicit compiler-owned bounds.
    /// A source binder or a foreign snapshot never inherits catalog authority.
    pub(super) fn declared_sized_bound(&self, parameter: ParamId) -> Result<Option<bool>> {
        charged(&self.budget, self.budget.charge_work(1))?;
        let Some(owner) = self.declaration(parameter.owner.declaration) else {
            return Ok(None);
        };
        if owner.origin
            != (DeclOrigin::CoreCatalog {
                version: CATALOG_VERSION,
                item: CatalogItem::ManuallyDrop,
            })
        {
            return Ok(None);
        }
        if owner.declaration != self.id(CatalogItem::ManuallyDrop)
            || owner.info.kind != DeclKind::Struct
            || owner.info.owner.is_some()
            || owner.info.own_env.binder().declaration != owner.declaration
            || owner.info.own_env.arity() != 1
            || owner.info.env.total_arity() != 1
            || !owner.info.own_env.contains(parameter)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "catalog declared bound changed canonical owner or parameter binder",
            ));
        }
        for predicate in self.declared_bounds.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if let Predicate::Implements(goal) = predicate
                && goal.declaration == self.id(CatalogItem::Sized)
                && goal.args.is_empty()
                && matches!(&goal.self_ty, Ty::Param(actual) if *actual == parameter)
            {
                return Ok(Some(true));
            }
        }
        Ok(Some(false))
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
    Ok(DeclId {
        snapshot,
        index: DeclId::SOURCE_BIT | u64::from(index),
    })
}

#[cfg(test)]
mod wide_tests {
    use super::*;

    #[test]
    fn catalog_declared_sized_bound_rejects_foreign_owner_wrong_binder_and_forged_rows() {
        let budget = PlanBudget::default();
        let modules = crate::compiler::module_identity::ModuleArenaBuilder::for_catalog(
            crate::compiler::expansion_budget::ExpansionBudget::default(),
            crate::compiler::owner_path::standalone_issued(
                &crate::compiler::expansion_budget::ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap();
        let snapshot = SnapshotId::standalone().unwrap();
        let builder = CoreCatalogBuilder::new(snapshot, modules, &budget).unwrap();
        let start = builder.source_start();
        let mut catalog = builder
            .finish(start, RecipeShapeInventory::new(), &budget)
            .unwrap();
        let parameter = ParamId {
            owner: BinderId {
                declaration: catalog.id(CatalogItem::ManuallyDrop),
            },
            index: 0,
        };
        assert_eq!(catalog.declared_sized_bound(parameter).unwrap(), Some(true));
        assert!(
            catalog
                .declared_sized_bound(ParamId {
                    index: 1,
                    ..parameter
                })
                .is_err()
        );
        let foreign = ParamId {
            owner: BinderId {
                declaration: DeclId {
                    snapshot: SnapshotId::standalone().unwrap(),
                    index: parameter.owner.declaration.index,
                },
            },
            ..parameter
        };
        assert_eq!(catalog.declared_sized_bound(foreign).unwrap(), None);
        for item in [
            CatalogItem::PhantomData,
            CatalogItem::Copy,
            CatalogItem::ManuallyDropNew,
        ] {
            assert_eq!(
                catalog
                    .declared_sized_bound(ParamId {
                        owner: BinderId {
                            declaration: catalog.id(item)
                        },
                        index: 0
                    })
                    .unwrap(),
                None
            );
        }
        let wrong_trait = catalog.id(CatalogItem::Copy);
        Arc::get_mut(&mut catalog).unwrap().declared_bounds =
            Arc::from([Predicate::Implements(TraitRef {
                declaration: wrong_trait,
                self_ty: Ty::Param(parameter),
                args: Arc::from([]),
            })]);
        assert_eq!(
            catalog.declared_sized_bound(parameter).unwrap(),
            Some(false)
        );
        let sized = catalog.id(CatalogItem::Sized);
        Arc::get_mut(&mut catalog).unwrap().declared_bounds =
            Arc::from([Predicate::Implements(TraitRef {
                declaration: sized,
                self_ty: Ty::Param(foreign),
                args: Arc::from([]),
            })]);
        assert_eq!(
            catalog.declared_sized_bound(parameter).unwrap(),
            Some(false)
        );
        Arc::get_mut(&mut catalog).unwrap().declared_bounds = Arc::from([]);
        assert_eq!(
            catalog.declared_sized_bound(parameter).unwrap(),
            Some(false)
        );
    }
    #[test]
    fn manually_drop_operations_keep_owning_catalog_binder_and_exclusive_drop_signature() {
        let budget = PlanBudget::default();
        let modules = crate::compiler::module_identity::ModuleArenaBuilder::for_catalog(
            crate::compiler::expansion_budget::ExpansionBudget::default(),
            crate::compiler::owner_path::standalone_issued(
                &crate::compiler::expansion_budget::ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap();
        let snapshot = SnapshotId::standalone().unwrap();
        let builder = CoreCatalogBuilder::new(snapshot, modules, &budget).unwrap();
        for item in [
            CatalogItem::ManuallyDropNew,
            CatalogItem::ManuallyDropIntoInner,
            CatalogItem::ManuallyDropDrop,
        ] {
            let entry = builder
                .base_declarations()
                .iter()
                .find(|entry| entry.declaration == id(snapshot, item))
                .unwrap();
            assert_eq!(entry.info.kind, DeclKind::AssociatedFunction);
            assert_eq!(
                entry.info.owner,
                Some(id(snapshot, CatalogItem::ManuallyDrop))
            );
            assert_eq!(entry.info.own_env.arity(), 0);
            assert_eq!(entry.info.env.total_arity(), 1);
            let schema = entry.schema.as_ref().unwrap();
            assert_eq!(schema.signature.params.len(), 1);
            assert!(
                matches!(&schema.predicates[0],Predicate::Implements(bound) if bound.declaration == id(snapshot,CatalogItem::Sized) && bound.args.is_empty())
            );
            match item {
                CatalogItem::ManuallyDropNew => assert!(
                    matches!(&schema.signature.result, Ty::ManuallyDrop(wrapper) if matches!(wrapper.argument(),Ty::Param(p) if p.owner.declaration == id(snapshot,CatalogItem::ManuallyDrop)))
                ),
                CatalogItem::ManuallyDropIntoInner => {
                    assert!(matches!(&schema.signature.params[0], Ty::ManuallyDrop(_)))
                }
                CatalogItem::ManuallyDropDrop => {
                    assert!(
                        matches!(&schema.signature.params[0],Ty::Ref {kind: BorrowKind::Mut,referent} if matches!(referent.as_ref(),Ty::ManuallyDrop(_)))
                    );
                    assert_eq!(schema.signature.result, Ty::Unit);
                }
                _ => unreachable!(),
            }
        }
    }
    #[test]
    fn catalog_retains_same_checked_module_arena_and_typed_paths() {
        let budget = PlanBudget::default();
        let modules = crate::compiler::module_identity::ModuleArenaBuilder::for_catalog(
            crate::compiler::expansion_budget::ExpansionBudget::default(),
            crate::compiler::owner_path::standalone_issued(
                &crate::compiler::expansion_budget::ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap();
        let snapshot = SnapshotId::standalone().unwrap();
        let builder = CoreCatalogBuilder::new(snapshot, modules.clone(), &budget).unwrap();
        assert!(Arc::ptr_eq(builder.modules(), &modules));
        for declaration in builder.base_declarations() {
            let DeclOrigin::CoreCatalog { version, item } = declaration.origin else {
                panic!("expected catalog origin")
            };
            assert_eq!(version, CATALOG_VERSION);
            assert_eq!(
                declaration.info.module,
                modules.catalog_module(item.module())
            );
            declaration
                .info
                .visibility
                .validate_for(&modules, declaration.info.module)
                .unwrap();
        }
        let factory = ir::PhantomDataFactory::from_builder(&builder, &budget).unwrap();
        let marker_schema = recipe_schema(
            DeclId {
                snapshot,
                index: 100,
            },
            RecipeDescriptor::ClonePhantomData,
            id(snapshot, CatalogItem::Sized),
            id(snapshot, CatalogItem::Clone),
            Some(factory),
            &budget,
        )
        .unwrap();
        let Ty::PhantomData(marker) = &marker_schema.signature.result else {
            panic!("marker clone must return its canonical descriptor");
        };
        assert_eq!(marker.declaration(), id(snapshot, CatalogItem::PhantomData));
        assert_eq!(marker_schema.env.ordered_params().count(), 1);
        assert_eq!(marker_schema.predicates.len(), 1);
        assert!(
            matches!(&marker_schema.predicates[0], Predicate::Implements(goal)
            if goal.declaration == id(snapshot, CatalogItem::Sized)
                && &goal.self_ty == marker.argument())
        );
        assert!(matches!(marker_schema.signature.params.as_slice(),
            [Ty::Ref { kind: BorrowKind::Shared, referent }]
                if referent.as_ref() == &marker_schema.signature.result));
        assert!(
            recipe_schema(
                DeclId {
                    snapshot,
                    index: 100
                },
                RecipeDescriptor::ClonePhantomData,
                id(snapshot, CatalogItem::Sized),
                id(snapshot, CatalogItem::Clone),
                None,
                &budget,
            )
            .is_err()
        );
        let clone_from_schema = recipe_schema(
            DeclId {
                snapshot,
                index: 101,
            },
            RecipeDescriptor::CloneFromPhantomData,
            id(snapshot, CatalogItem::Sized),
            id(snapshot, CatalogItem::Clone),
            Some(factory),
            &budget,
        )
        .unwrap();
        assert_eq!(clone_from_schema.signature.result, Ty::Unit);
        assert_eq!(clone_from_schema.predicates.len(), 1);
        assert!(
            matches!(&clone_from_schema.predicates[0], Predicate::Implements(goal)
            if goal.declaration == id(snapshot, CatalogItem::Sized))
        );
        let [
            Ty::Ref {
                kind: BorrowKind::Mut,
                referent: destination,
            },
            Ty::Ref {
                kind: BorrowKind::Shared,
                referent: source,
            },
        ] = clone_from_schema.signature.params.as_slice()
        else {
            panic!("marker clone_from must retain mutable destination and shared source");
        };
        assert_eq!(destination, source);
        assert!(matches!(destination.as_ref(), Ty::PhantomData(marker)
            if marker.declaration() == id(snapshot, CatalogItem::PhantomData)
                && matches!(marker.argument(), Ty::Param(parameter)
                    if parameter.owner.declaration.index == 101)));
        let source_end = builder.source_start();
        let mut inventory = RecipeShapeInventory::new();
        for descriptor in [
            RecipeDescriptor::ClonePhantomData,
            RecipeDescriptor::CloneFromPhantomData,
        ] {
            inventory
                .record(
                    descriptor,
                    &super::super::super::ownership::RecipePlanCounts::default(),
                    &budget,
                )
                .unwrap();
        }
        let catalog = builder.finish(source_end, inventory, &budget).unwrap();
        for (descriptor, kind) in [
            (RecipeDescriptor::ClonePhantomData, BorrowKind::Shared),
            (RecipeDescriptor::CloneFromPhantomData, BorrowKind::Mut),
        ] {
            let recipe = catalog.recipe(descriptor).unwrap();
            assert_eq!(recipe.info.receiver, ReceiverMode::Borrowed { kind });
            assert!(
                matches!(recipe.schema.as_ref().unwrap().signature.params.first(),
                Some(Ty::Ref { kind: actual, .. }) if *actual == kind)
            );
        }
        assert!(Arc::ptr_eq(catalog.modules(), &modules));
        assert_eq!(CatalogItem::Sized.binding_atom(), Some(BuiltinAtom::Sized));
        assert_eq!(CatalogItem::CloneItem.binding_atom(), None);
        assert!(matches!(
            catalog.modules().source_root(),
            Err(ModuleError::SourceUnavailable)
        ));
    }

    #[test]
    fn catalog_preserves_poisoned_expansion_authority_as_resource() {
        let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
        let modules = crate::compiler::module_identity::ModuleArenaBuilder::for_catalog(
            expansion.clone(),
            crate::compiler::owner_path::standalone_issued(
                &expansion.clone(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap();
        assert!(
            expansion
                .charge(crate::compiler::expansion_budget::Counter::Work, u64::MAX)
                .is_err()
        );
        let budget = PlanBudget::default();
        assert!(
            matches!(CoreCatalogBuilder::new(SnapshotId::standalone().unwrap(), modules, &budget), Err(error) if error.is_resource())
        );
    }

    #[test]
    fn char_clone_schema_and_identity_are_distinct() {
        let budget = PlanBudget::default();
        let snapshot = SnapshotId::standalone().unwrap();
        let schema = recipe_schema(
            DeclId {
                snapshot,
                index: 100,
            },
            RecipeDescriptor::CloneChar,
            id(snapshot, CatalogItem::Sized),
            id(snapshot, CatalogItem::Clone),
            None,
            &budget,
        )
        .unwrap();
        assert_eq!(schema.signature.result, Ty::Char);
        assert!(
            matches!(schema.signature.params.as_slice(), [Ty::Ref { kind: BorrowKind::Shared, referent }] if **referent == Ty::Char)
        );
        assert!(schema.predicates.is_empty());
        assert_eq!(schema.env.ordered_params().count(), 0);
        assert_eq!(RecipeDescriptor::CloneChar.order(), (9, 0));
        assert!(
            RecipeDescriptor::CloneInt(IntTy::U128).order() < RecipeDescriptor::CloneChar.order()
        );
    }

    #[test]
    fn wide_clone_catalog_schema_preserves_signedness_and_scalar_result() {
        let budget = PlanBudget::new(ir::PlanLimits::default());
        let snapshot = SnapshotId::standalone().unwrap();
        for kind in [IntTy::I128, IntTy::U128] {
            let declaration = DeclId {
                snapshot,
                index: 100,
            };
            let schema = recipe_schema(
                declaration,
                RecipeDescriptor::CloneInt(kind),
                id(snapshot, CatalogItem::Sized),
                id(snapshot, CatalogItem::Clone),
                None,
                &budget,
            )
            .unwrap();
            assert!(matches!(schema.signature.result, Ty::Int(actual) if actual == kind));
            assert!(
                matches!(schema.signature.params.as_slice(), [Ty::Ref { kind: BorrowKind::Shared, referent }] if matches!(referent.as_ref(), Ty::Int(actual) if *actual == kind))
            );
            assert!(schema.signature.ref_contract.results.is_empty());
            assert_eq!(schema.signature.ref_contract.input_regions.len(), 1);
            assert!(schema.predicates.is_empty());
            assert_eq!(schema.env.ordered_params().count(), 0);
        }
    }

    #[test]
    fn wide_recipe_identity_appends_without_legacy_order_drift() {
        let ints = [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
        ];
        for (index, kind) in ints.into_iter().enumerate() {
            assert_eq!(RecipeDescriptor::CloneInt(kind).order(), (0, index as u16));
        }
        let old = [
            (RecipeDescriptor::CloneBool, (1, 0)),
            (RecipeDescriptor::CloneUnit, (2, 0)),
            (RecipeDescriptor::CloneNever, (3, 0)),
            (RecipeDescriptor::CloneTuple { arity: 2 }, (4, 2)),
            (RecipeDescriptor::CloneArray { len: 3 }, (5, 3)),
            (RecipeDescriptor::CloneSharedRef, (6, 0)),
            (RecipeDescriptor::CloneFloat(ir::FloatTy::F32), (7, 0)),
            (RecipeDescriptor::CloneFloat(ir::FloatTy::F64), (7, 1)),
        ];
        for (descriptor, order) in old {
            assert_eq!(descriptor.order(), order);
            assert!(order < RecipeDescriptor::CloneInt(IntTy::I128).order());
        }
        assert_eq!(RecipeDescriptor::CloneInt(IntTy::I128).order(), (8, 0));
        assert_eq!(RecipeDescriptor::CloneInt(IntTy::U128).order(), (8, 1));
        assert_ne!(
            RecipeDescriptor::CloneInt(IntTy::I128),
            RecipeDescriptor::CloneInt(IntTy::U128)
        );
    }
}
