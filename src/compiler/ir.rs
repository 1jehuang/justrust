//! Checked scalar, tuple and nominal struct bodies shared by analysis and codegen.
//!
//! IDs are local to one immutable function, not persistent cache identities.
//! Allocate a new LocalId for every binding, including shadowing. Names are for
//! diagnostics only. This structured representation is a migration boundary,
//! not yet an ownership-checking CFG or a target layout/ABI representation.
use super::frontend::{BinaryOp, UnaryOp};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local namespace for one validated declaration environment. IDs must
/// not be persisted as cache identities or reused by another environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SnapshotId(u64);

impl SnapshotId {
    pub fn allocate() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self::allocate_from(&NEXT)
    }

    fn allocate_from(next: &AtomicU64) -> Result<Self, String> {
        let mut id = next.load(Ordering::Relaxed);
        loop {
            let successor = id
                .checked_add(1)
                .ok_or("nominal type snapshot IDs exhausted")?;
            match next.compare_exchange_weak(id, successor, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Ok(Self(id)),
                Err(observed) => id = observed,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DefId {
    pub snapshot: SnapshotId,
    pub index: usize,
}

/// Immutable nominal handle. Equality and hashing use declaration identity,
/// never field shape, names or Arc's structural pointee equality.
#[derive(Clone)]
pub struct StructTy(Arc<StructDef>);

impl std::fmt::Debug for StructTy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Types are a shared DAG, not a tree to expand into diagnostics. Names
        // themselves can be long, so bound their diagnostic rendering as well.
        let name = &self.0.name;
        let end = name
            .char_indices()
            .nth(128)
            .map_or(name.len(), |(at, _)| at);
        f.debug_struct("StructTy")
            .field("id", &self.0.id)
            .field("name", &&name[..end])
            .finish_non_exhaustive()
    }
}

impl StructTy {
    /// The declaration builder must validate the entire by-value dependency DAG
    /// before constructing definitions in dependency-first order.
    pub(super) fn new(def: StructDef) -> Self {
        Self(Arc::new(def))
    }

    pub fn def(&self) -> &StructDef {
        &self.0
    }

    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.0.fields.iter().position(|field| field.name == name)
    }
}

impl PartialEq for StructTy {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for StructTy {}

impl Hash for StructTy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub id: DefId,
    /// Canonical module-qualified name, used for diagnostics, not identity.
    pub name: String,
    /// Defining module, used for owner/descendant field visibility checks.
    pub module: String,
    pub public: bool,
    /// Declaration order. No layout offsets or implicit Copy permission.
    pub fields: Arc<[FieldDef]>,
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub public: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(pub usize);
/// Resolved control-flow target, allocated monotonically within one function.
/// Labels are not represented by this milestone's source language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoopId(pub usize);

/// Resolved semantic types. Inference variables never cross this boundary.
/// Tuple structure is immutable and shared cheaply across checked facts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    I32,
    Bool,
    Unit,
    /// Nonempty structural tuple. The empty tuple is canonically Unit.
    /// Rust Copy semantics for admitted tuples do not make this host enum Copy.
    Tuple(Arc<[Ty]>),
    /// Nominal and always non-Copy in this slice, even with only scalar fields.
    Struct(StructTy),
    /// The semantic never type, distinct from runtime completion information.
    Never,
}

impl Ty {
    pub fn field_count(&self) -> Option<usize> {
        match self {
            Self::Tuple(fields) => Some(fields.len()),
            Self::Struct(ty) => Some(ty.def().fields.len()),
            _ => None,
        }
    }

    pub fn field_ty(&self, index: usize) -> Option<&Ty> {
        match self {
            Self::Tuple(fields) => fields.get(index),
            Self::Struct(ty) => ty.def().fields.get(index).map(|field| &field.ty),
            _ => None,
        }
    }

    pub fn is_copy(&self) -> bool {
        match self {
            Self::I32 | Self::Bool | Self::Unit | Self::Never => true,
            Self::Tuple(fields) => fields.iter().all(Self::is_copy),
            Self::Struct(_) => false,
        }
    }
}

/// Static normal-completion summary, independent of semantic value type.
/// Returns is conservative: it does not prove an exit is dynamically reachable
/// or that evaluation terminates. Never does not erase an explicit tail's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Returns,
    Never,
}

#[derive(Clone, Debug)]
pub struct LocalDecl {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub params: Vec<Ty>,
    pub result: Ty,
}

#[derive(Clone, Debug)]
pub struct CheckedFunction {
    pub name: String,
    /// Ordered parameter bindings, distinct entries in `locals`.
    pub params: Vec<LocalId>,
    pub result: Ty,
    pub locals: Vec<LocalDecl>,
    pub exprs: Vec<TypedExpr>,
    pub body: Block,
}

#[derive(Clone, Debug)]
pub struct TypedExpr {
    /// Actual semantic type, independent of whether evaluation completes.
    /// For example, `{ return; 0 }` has type I32 and completion Never, while
    /// `{ return; }` has type Never and completion Never. Never is not a
    /// placeholder for every expression that cannot complete normally.
    pub ty: Ty,
    pub completion: Completion,
    pub kind: ExprKind,
}
impl TypedExpr {
    /// Normal-completion summary only, not a semantic type query. In particular,
    /// None discards the actual type of an unreachable but explicitly typed tail.
    /// Operator selection, coercion and inference must inspect `ty` separately.
    pub fn flow(&self) -> Option<Ty> {
        match self.completion {
            Completion::Returns => Some(self.ty.clone()),
            Completion::Never => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub tail: Option<ExprId>,
}

/// A resolved local-rooted storage location. Field indices are in source
/// projection order and are checked against tuple/struct types by typing.
/// Empty fields denote the whole local. No byte offsets or target ABI here.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub local: LocalId,
    pub fields: Vec<usize>,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        local: LocalId,
        value: ExprId,
    },
    Assign {
        local: LocalId,
        value: ExprId,
    },
    AssignPlace {
        place: Place,
        value: ExprId,
    },
    Expr(ExprId),
    Return(Option<ExprId>),
    While {
        id: LoopId,
        condition: ExprId,
        body: Block,
    },
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i32),
    Bool(bool),
    /// Read a resolved local into this expression's independent value snapshot.
    Local(LocalId),
    /// Elements are evaluated left to right into a new value snapshot.
    Tuple(Vec<ExprId>),
    /// Initializers retain source evaluation order. Each index names a field in
    /// declaration order; lowering permutes snapshots, never source evaluation.
    Struct {
        ty: StructTy,
        fields: Vec<(usize, ExprId)>,
    },
    /// Projects from an arbitrary expression snapshot, not a live local alias.
    Field {
        base: ExprId,
        index: usize,
    },
    Unary {
        op: UnaryOp,
        value: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Call {
        name: String,
        args: Vec<ExprId>,
        /// Validated at typing time, shared by call lowering and ABI selection.
        signature: Signature,
    },
    If {
        condition: ExprId,
        then_block: Block,
        else_block: Block,
    },
    Block(Block),
    /// Repeats the body until an associated break produces a value.
    Loop {
        id: LoopId,
        body: Block,
    },
    /// Evaluates the operand before transferring to the resolved target's exit.
    /// This expression itself never completes normally, even for a unit break.
    Break {
        target: LoopId,
        value: Option<ExprId>,
    },
    /// Transfers to the target's head, including condition evaluation for while.
    Continue {
        target: LoopId,
    },
    Println(String),
}

impl CheckedFunction {
    /// Optional structural debugging check. This is not source type checking,
    /// borrow checking, or a codegen prerequisite. Normal compilation consumes
    /// the immutable result produced by the checker without revalidating it.
    ///
    /// Checks reference bounds, distinct parameters, and call arity. Deliberately
    /// does not establish lexical scope, reachability, expression flow, or loop
    /// target validity or projection typing. Those are established once by
    /// semantic analysis.
    pub fn validate(&self) -> Result<(), String> {
        let local = |id: LocalId| {
            if id.0 < self.locals.len() {
                Ok(())
            } else {
                Err(format!("invalid local {} in {}", id.0, self.name))
            }
        };
        let expr = |id: ExprId| {
            if id.0 < self.exprs.len() {
                Ok(())
            } else {
                Err(format!("invalid expression {} in {}", id.0, self.name))
            }
        };
        let mut parameters = std::collections::HashSet::new();
        for &id in &self.params {
            local(id)?;
            if !parameters.insert(id) {
                return Err(format!(
                    "duplicate parameter local {} in {}",
                    id.0, self.name
                ));
            }
        }
        // Traverse nested blocks iteratively so malformed/deep fixtures cannot
        // exhaust the host call stack inside this diagnostic helper.
        let mut blocks = vec![&self.body];
        for value in &self.exprs {
            match &value.kind {
                ExprKind::Local(id) => local(*id)?,
                ExprKind::Tuple(elements) => {
                    for &id in elements {
                        expr(id)?;
                    }
                }
                ExprKind::Struct { fields, .. } => {
                    for &(_, id) in fields {
                        expr(id)?;
                    }
                }
                ExprKind::Field { base, .. } => expr(*base)?,
                ExprKind::Unary { value, .. } => expr(*value)?,
                ExprKind::Binary { left, right, .. } => {
                    expr(*left)?;
                    expr(*right)?;
                }
                ExprKind::Call {
                    name,
                    args,
                    signature,
                } => {
                    if args.len() != signature.params.len() {
                        return Err(format!("invalid call arity for {name} in {}", self.name));
                    }
                    for &id in args {
                        expr(id)?;
                    }
                }
                ExprKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    expr(*condition)?;
                    blocks.push(then_block);
                    blocks.push(else_block);
                }
                ExprKind::Block(block) | ExprKind::Loop { body: block, .. } => blocks.push(block),
                ExprKind::Break {
                    value: Some(value), ..
                } => expr(*value)?,
                ExprKind::Break { value: None, .. } | ExprKind::Continue { .. } => {}
                ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Println(_) => {}
            }
        }
        while let Some(block) = blocks.pop() {
            if let Some(id) = block.tail {
                expr(id)?;
            }
            for statement in &block.statements {
                match statement {
                    Stmt::Let { local: id, value } | Stmt::Assign { local: id, value } => {
                        local(*id)?;
                        expr(*value)?;
                    }
                    Stmt::AssignPlace { place, value } => {
                        local(place.local)?;
                        expr(*value)?;
                    }
                    Stmt::Expr(id) | Stmt::Return(Some(id)) => expr(*id)?,
                    Stmt::Return(None) => {}
                    Stmt::While {
                        condition, body, ..
                    } => {
                        expr(*condition)?;
                        blocks.push(body);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nominal(id: DefId, name: &str, fields: Vec<Ty>) -> StructTy {
        StructTy::new(StructDef {
            id,
            name: name.into(),
            module: String::new(),
            public: false,
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| FieldDef {
                    name: format!("f{index}"),
                    ty,
                    public: false,
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    #[test]
    fn nominal_identity_is_not_name_shape_or_declaration_index_alone() {
        let snapshot = SnapshotId::allocate().unwrap();
        let first = nominal(DefId { snapshot, index: 0 }, "S", vec![Ty::I32]);
        let same_shape = nominal(DefId { snapshot, index: 1 }, "S", vec![Ty::I32]);
        let next_snapshot = nominal(
            DefId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            "S",
            vec![Ty::I32],
        );
        assert_eq!(first, first.clone());
        assert_ne!(first, same_shape);
        assert_ne!(first, next_snapshot);
        let mut identities = std::collections::HashSet::new();
        assert!(identities.insert(first.clone()));
        assert!(!identities.insert(first));
        assert!(identities.insert(same_shape));
        assert!(identities.insert(next_snapshot));
    }

    #[test]
    fn aggregate_helpers_preserve_nominal_non_copy_and_projected_copy() {
        let handle = nominal(
            DefId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            "S",
            vec![Ty::I32, Ty::Tuple(vec![Ty::Bool, Ty::Unit].into())],
        );
        assert_eq!(handle.field_index("f1"), Some(1));
        assert_eq!(handle.field_index("missing"), None);
        let ty = Ty::Struct(handle);
        assert_eq!(ty.field_count(), Some(2));
        assert_eq!(ty.field_ty(0), Some(&Ty::I32));
        assert_eq!(ty.field_ty(2), None);
        assert!(!ty.is_copy());
        assert!(ty.field_ty(1).unwrap().is_copy());
        assert!(Ty::Tuple(vec![Ty::Bool, Ty::Unit].into()).is_copy());
        assert!(!Ty::Tuple(vec![Ty::Bool, ty].into()).is_copy());
        assert_eq!(Ty::I32.field_count(), None);
        assert_eq!(Ty::I32.field_ty(0), None);
    }

    #[test]
    fn snapshot_allocation_fails_at_exhaustion_without_wrapping() {
        let next = AtomicU64::new(u64::MAX - 1);
        assert_eq!(SnapshotId::allocate_from(&next).unwrap().0, u64::MAX - 1);
        assert!(SnapshotId::allocate_from(&next).is_err());
        assert!(SnapshotId::allocate_from(&next).is_err());
        assert_eq!(next.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn snapshot_allocation_is_unique_across_threads() {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..32)
                        .map(|_| SnapshotId::allocate().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let ids: std::collections::HashSet<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 256);
    }

    #[test]
    fn nominal_debug_bounds_unicode_names_and_does_not_expand_shared_fields() {
        let snapshot = SnapshotId::allocate().unwrap();
        let mut ty = Ty::I32;
        for index in 0..16 {
            ty = Ty::Struct(nominal(
                DefId { snapshot, index },
                &"é".repeat(1000),
                vec![ty.clone(), ty],
            ));
        }
        let diagnostic = format!("{ty:?}");
        assert!(diagnostic.len() < 512, "unbounded nominal diagnostic");
        assert!(!diagnostic.contains("fields"));
        assert!(diagnostic.contains("DefId"));
    }

    #[test]
    fn tuple_types_are_structural_and_cheaply_cloned() {
        let fields: Arc<[Ty]> = vec![Ty::I32, Ty::Tuple(vec![Ty::Bool, Ty::Unit].into())].into();
        let ty = Ty::Tuple(fields.clone());
        let expression = TypedExpr {
            ty: ty.clone(),
            completion: Completion::Returns,
            kind: ExprKind::Tuple(vec![]),
        };
        let Some(Ty::Tuple(cloned_fields)) = expression.flow() else {
            panic!("tuple flow missing")
        };
        assert!(Arc::ptr_eq(&fields, &cloned_fields));
        assert_eq!(
            ty,
            Ty::Tuple(vec![Ty::I32, Ty::Tuple(vec![Ty::Bool, Ty::Unit].into())].into())
        );
        assert_ne!(ty, Ty::Tuple(vec![Ty::Bool, Ty::I32].into()));
        assert_ne!(Ty::Unit, Ty::Tuple(vec![Ty::Unit].into()));
    }

    #[test]
    fn tuple_field_and_assignment_references_are_checked() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Tuple(vec![Ty::I32].into()),
            completion: Completion::Returns,
            kind: ExprKind::Tuple(vec![ExprId(0)]),
        });
        value.exprs.push(TypedExpr {
            ty: Ty::I32,
            completion: Completion::Returns,
            kind: ExprKind::Field {
                base: ExprId(1),
                index: 0,
            },
        });
        value.body.statements.push(Stmt::AssignPlace {
            place: Place {
                local: LocalId(0),
                fields: vec![0, 1],
            },
            value: ExprId(2),
        });
        // Structural validation intentionally does not redo projection typing.
        value.validate().unwrap();
        value.exprs[2].kind = ExprKind::Field {
            base: ExprId(99),
            index: 0,
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[2].kind = ExprKind::Tuple(vec![ExprId(99)]);
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[2].kind = ExprKind::Int(0);
        value.body.statements.push(Stmt::AssignPlace {
            place: Place {
                local: LocalId(99),
                fields: vec![0],
            },
            value: ExprId(2),
        });
        assert!(value.validate().unwrap_err().contains("invalid local 99"));
    }

    fn fixture() -> CheckedFunction {
        CheckedFunction {
            name: "f".into(),
            params: vec![LocalId(0)],
            result: Ty::Unit,
            locals: vec![
                LocalDecl {
                    name: "x".into(),
                    ty: Ty::I32,
                    mutable: false,
                },
                LocalDecl {
                    name: "x".into(),
                    ty: Ty::Bool,
                    mutable: true,
                },
            ],
            exprs: vec![TypedExpr {
                ty: Ty::I32,
                completion: Completion::Returns,
                kind: ExprKind::Local(LocalId(0)),
            }],
            body: Block {
                statements: vec![Stmt::Expr(ExprId(0))],
                tail: None,
            },
        }
    }

    #[test]
    fn divergence_is_not_unit_flow() {
        let value = TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Block(Block {
                statements: vec![Stmt::Return(None)],
                tail: None,
            }),
        };
        assert_eq!(value.flow(), None);
        assert_eq!(value.ty, Ty::Never);
        let value = TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            ..value
        };
        assert_eq!(value.flow(), Some(Ty::Unit));
    }

    #[test]
    fn unreachable_typed_tail_retains_its_semantic_type() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::I32,
            completion: Completion::Never,
            kind: ExprKind::Block(Block {
                statements: vec![Stmt::Return(None)],
                tail: Some(ExprId(0)),
            }),
        });
        assert_eq!(value.exprs[1].ty, Ty::I32);
        assert_eq!(value.exprs[1].flow(), None);
        assert_ne!(value.exprs[1].ty, Ty::Never);
        value.validate().unwrap();

        // A loop's break operand can have a concrete type without completing.
        // Its semantic result type must not be overwritten by its completion.
        value.exprs.push(TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Break {
                target: LoopId(0),
                value: Some(ExprId(1)),
            },
        });
        value.exprs.push(TypedExpr {
            ty: Ty::I32,
            completion: Completion::Never,
            kind: ExprKind::Loop {
                id: LoopId(0),
                body: Block {
                    statements: vec![Stmt::Expr(ExprId(2))],
                    tail: None,
                },
            },
        });
        assert_eq!(value.exprs[3].ty, Ty::I32);
        assert_eq!(value.exprs[3].flow(), None);
        value.validate().unwrap();
    }

    #[test]
    fn display_names_do_not_define_local_identity() {
        let value = fixture();
        assert_eq!(value.locals[0].name, value.locals[1].name);
        assert_ne!(LocalId(0), LocalId(1));
        value.validate().unwrap();
    }

    #[test]
    fn structural_validation_rejects_invalid_references_and_parameters() {
        let mut value = fixture();
        value.body.tail = Some(ExprId(1));
        assert!(value.validate().unwrap_err().contains("invalid expression"));
        value.body.tail = None;
        value.params.push(LocalId(0));
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("duplicate parameter")
        );
        value.params = vec![LocalId(2)];
        assert!(value.validate().unwrap_err().contains("invalid local"));
    }

    #[test]
    fn structural_validation_rejects_call_arity() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            kind: ExprKind::Call {
                name: "g".into(),
                args: vec![],
                signature: Signature {
                    params: vec![Ty::I32],
                    result: Ty::Unit,
                },
            },
        });
        assert!(value.validate().unwrap_err().contains("invalid call arity"));
    }

    #[test]
    fn loop_body_and_break_operand_references_are_checked() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Loop {
                id: LoopId(0),
                body: Block {
                    statements: vec![],
                    tail: Some(ExprId(99)),
                },
            },
        });
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[1].kind = ExprKind::Break {
            target: LoopId(0),
            value: Some(ExprId(99)),
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[1].kind = ExprKind::Break {
            target: LoopId(0),
            value: Some(ExprId(0)),
        };
        assert_eq!(value.exprs[1].flow(), None);
        value.validate().unwrap();
        value.exprs[1].kind = ExprKind::Continue { target: LoopId(0) };
        assert_eq!(value.exprs[1].flow(), None);
        value.validate().unwrap();
    }

    #[test]
    fn while_body_references_are_checked() {
        let mut value = fixture();
        value.body.statements.push(Stmt::While {
            id: LoopId(0),
            condition: ExprId(0),
            body: Block {
                statements: vec![Stmt::Expr(ExprId(99))],
                tail: None,
            },
        });
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
    }
}
