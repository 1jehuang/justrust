//! Source checking, local inference and checked body construction.
use super::*;

#[path = "type_check/associated_hint.rs"]
mod associated_hint;
#[path = "type_check/binary_expression.rs"]
mod binary_expression;

mod array_iterator;
mod byte_strings;
#[path = "type_check/callable_arguments.rs"]
mod callable_arguments;
#[path = "type_check/constructor_expectations.rs"]
mod constructor_expectations;
#[path = "type_check/fn_item_values.rs"]
mod fn_item_values;
#[path = "format_lowering.rs"]
mod format_lowering;
#[path = "type_check/generic_placeholders.rs"]
mod generic_placeholders;
#[path = "type_check/inferred_owner.rs"]
mod inferred_owner;
#[path = "type_check/inherent_method_hint.rs"]
mod inherent_method_hint;
mod iterator_loop;
#[path = "local_literals.rs"]
mod local_literals;
pub(in crate::compiler) use local_literals::nominal_manifest::{
    DeclarationFields as NominalDeclarationFields, DeclarationRecipe as NominalDeclarationRecipe,
    Demand as NominalDemand, Inputs as NominalPreparationInputs, Preparation as NominalPreparation,
    PreparedHints as NominalPreparedHints, Shape as NominalShape,
};
#[path = "type_check/method_argument_hint.rs"]
mod method_argument_hint;
#[path = "type_check/method_owner.rs"]
mod method_owner;
mod nominal_calls;
pub(in crate::compiler::frontend) mod nominal_inputs;
#[path = "type_check/opaque_returns.rs"]
mod opaque_returns;
#[path = "type_check/partial_annotation.rs"]
pub(super) mod partial_annotation;
mod primitive_actuals;
#[path = "type_check/projected_formals.rs"]
mod projected_formals;
#[path = "type_check/shared_or_arms.rs"]
mod shared_or_arms;
#[path = "type_check/trait_method_hint.rs"]
mod trait_method_hint;
#[path = "try_operator.rs"]
mod try_operator;
#[path = "type_check/unsafety.rs"]
mod unsafety;

/// Borrowed source bodies and the semantic unit value of an absent else.
#[derive(Clone, Copy)]
enum MatchBody<'a> {
    Expr(&'a Expr),
    Block(&'a Block),
    ImplicitUnit,
    LoopExit(ir::LoopId),
}
#[derive(Clone, Copy)]
struct BorrowedMatchArm<'a> {
    /// None is an internal fallback, never a fabricated source pattern.
    pattern: Option<&'a Pattern>,
    guarded: bool,
    /// Ordinary match-arm guard, checked as a Bool in the arm's scope.
    guard: Option<&'a Expr>,
    body: MatchBody<'a>,
}

const UNEXPANDED_MACRO_CHECK_ERROR: &str =
    "unsupported: unexpanded macro syntax cannot enter active checking";

pub(super) fn scope_diagnostic(error: super::super::semantic_scope::ScopeError) -> String {
    super::super::semantic_scope::scope_diagnostic(error)
}

pub(super) fn application_namespace(value: &Expr) -> Result<Namespace, String> {
    match value {
        Expr::UnexpandedMacro(_) => Err(UNEXPANDED_MACRO_CHECK_ERROR.into()),
        Expr::Call { .. } => Ok(Namespace::Value),
        Expr::Struct { .. } | Expr::TupleStruct { .. } | Expr::EnumVariant { .. } => {
            Ok(Namespace::Type)
        }
        _ => Err("unsupported generic application expression".into()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum LocalKey {
    Named(NameKey),
    Anchored(super::super::semantic_scope::BindingId),
    Receiver,
}

pub(super) trait LocalInput {
    fn checked_key(&self, names: &NameStore) -> Result<Option<LocalKey>, String>;
    fn occurrence(&self, names: &NameStore) -> Result<Option<AstIdent>, String>;
    fn diagnostic(&self, names: &NameStore) -> Result<String, String>;
}
impl<T: LocalInput + ?Sized> LocalInput for &T {
    fn occurrence(&self, names: &NameStore) -> Result<Option<AstIdent>, String> {
        (**self).occurrence(names)
    }
    fn checked_key(&self, names: &NameStore) -> Result<Option<LocalKey>, String> {
        (**self).checked_key(names)
    }
    fn diagnostic(&self, names: &NameStore) -> Result<String, String> {
        (**self).diagnostic(names)
    }
}
impl LocalInput for AstIdent {
    fn occurrence(&self, names: &NameStore) -> Result<Option<AstIdent>, String> {
        names
            .check(*self)
            .map_err(native_adapter::name_diagnostic)?;
        Ok(Some(*self))
    }
    fn checked_key(&self, names: &NameStore) -> Result<Option<LocalKey>, String> {
        Ok(Some(LocalKey::Named(
            names
                .key(*self, LookupClass::Local)
                .map_err(native_adapter::name_diagnostic)?,
        )))
    }
    fn diagnostic(&self, names: &NameStore) -> Result<String, String> {
        Ok(names
            .spelling(*self)
            .map_err(native_adapter::name_diagnostic)?
            .to_owned())
    }
}
impl LocalInput for BindingName {
    fn occurrence(&self, names: &NameStore) -> Result<Option<AstIdent>, String> {
        match self {
            Self::Named(name) => name.occurrence(names),
            Self::Wildcard => Ok(None),
        }
    }
    fn checked_key(&self, names: &NameStore) -> Result<Option<LocalKey>, String> {
        match self {
            Self::Named(name) => name.checked_key(names),
            Self::Wildcard => Ok(None),
        }
    }
    fn diagnostic(&self, names: &NameStore) -> Result<String, String> {
        match self {
            Self::Named(name) => name.diagnostic(names),
            Self::Wildcard => Ok("_".into()),
        }
    }
}
impl LocalInput for LocalName {
    fn occurrence(&self, names: &NameStore) -> Result<Option<AstIdent>, String> {
        match self {
            Self::Named(name) => name.occurrence(names),
            Self::ReceiverOccurrence(name) => name.ident().occurrence(names),
            Self::Receiver => Ok(None),
        }
    }
    fn checked_key(&self, names: &NameStore) -> Result<Option<LocalKey>, String> {
        match self {
            Self::Named(name) => name.checked_key(names),
            Self::ReceiverOccurrence(name) => {
                let name = name.ident();
                names.check(name).map_err(native_adapter::name_diagnostic)?;
                if names
                    .spelling(name)
                    .map_err(native_adapter::name_diagnostic)?
                    != "self"
                {
                    return Err("receiver occurrence is not self".into());
                }
                Ok(Some(LocalKey::Receiver))
            }
            Self::Receiver => Ok(Some(LocalKey::Receiver)),
        }
    }
    fn diagnostic(&self, names: &NameStore) -> Result<String, String> {
        match self {
            Self::Named(name) => name.diagnostic(names),
            Self::Receiver | Self::ReceiverOccurrence(_) => Ok("self".into()),
        }
    }
}
impl LocalInput for AstPath {
    fn occurrence(&self, names: &NameStore) -> Result<Option<AstIdent>, String> {
        self.check(names).map_err(native_adapter::name_diagnostic)?;
        match (self.anchor(), self.segments()) {
            (PathAnchor::SelfModule, []) => Ok(self.receiver_occurrence().map(|name| name.ident())),
            (PathAnchor::Relative, [name]) => Ok(Some(*name)),
            _ => Err("expected a local-root path".into()),
        }
    }
    fn checked_key(&self, names: &NameStore) -> Result<Option<LocalKey>, String> {
        self.check(names).map_err(native_adapter::name_diagnostic)?;
        match (self.anchor(), self.segments()) {
            (PathAnchor::SelfModule, []) => Ok(Some(LocalKey::Receiver)),
            (PathAnchor::Relative, [name]) => name.checked_key(names),
            _ => Err("expected a local-root path".into()),
        }
    }
    fn diagnostic(&self, names: &NameStore) -> Result<String, String> {
        Ok(self
            .display(names)
            .map_err(native_adapter::name_diagnostic)?
            .to_string())
    }
}

#[derive(Clone, Debug)]
pub struct TypeEnv {
    pub structs: Arc<HashMap<PathKey, ir::StructTy>>,
    pub enums: Arc<HashMap<PathKey, ir::EnumTy>>,
    pub functions: Arc<HashMap<PathKey, ir::Signature>>,
    pub(super) plan: Option<PlanCtx>,
    pub(super) params: HashMap<GenericBindingKey, ir::Ty>,
    pub(super) param_env: Option<Arc<ir::GenericEnv>>,
    pub(super) self_ty: Option<ir::Ty>,
    pub(super) facts: Option<Arc<tr::ValidatedFacts>>,
}
impl TypeEnv {
    pub fn plan(&self) -> Result<&PlanCtx, String> {
        self.plan
            .as_ref()
            .ok_or_else(|| "missing authoritative plan".into())
    }
}
pub type Signatures = TypeEnv;
pub fn type_env(program: &Program) -> Result<TypeEnv, String> {
    let budget = ir::PlanBudget::default();
    PlanCtx::from_program(program, &budget)?.environment(None)
}
pub(super) struct LoopFrame {
    pub(super) id: ir::LoopId,
    pub(super) label: Option<NameKey>,
    pub(super) is_block: bool,
    pub(super) accepts_value: bool,
    pub(super) checking_condition: bool,
    pub(super) break_ty: Option<ir::Ty>,
    pub(super) expected: Option<ir::Ty>,
    pub(super) expected_is_inferred: bool,
    pub(super) expected_source_schema: Option<InferTerm>,
    pub(super) can_exit: bool,
}
pub(super) struct PatternLocal {
    pub(super) name: AstIdent,
    /// The binding local's own type: `&kind T` for a borrowing binding.
    pub(super) ty: ir::Ty,
    pub(super) mutable: bool,
    pub(super) projection: Vec<ir::Projection>,
    pub(super) borrow: Option<ir::BorrowKind>,
}

struct PendingPatternBinding {
    local: PatternLocal,
    key: NameKey,
    _projection_lease: super::super::expansion_budget::ByteLease,
}
struct PatternScratch {
    path: Vec<ir::Projection>,
    _lease: super::super::expansion_budget::ByteLease,
}
struct PendingPatternBindings {
    rows: super::super::token_session::BudgetVec<PendingPatternBinding>,
}
struct PendingDecisionTest {
    projection: Vec<ir::Projection>,
    kind: ir::PatternTestKind,
    _projection_lease: super::super::expansion_budget::ByteLease,
}
struct PendingDecision {
    tests: super::super::token_session::BudgetVec<PendingDecisionTest>,
    bindings: PendingPatternBindings,
    work: usize,
    refutable_payload: bool,
}
#[derive(Clone, Copy)]
enum PatternCoverage {
    CatchAll,
    Scalar(u128),
    Variant(usize),
    Predicate { root_variant: Option<usize> },
}
impl PatternCoverage {
    fn full_key(self) -> Option<u128> {
        match self {
            Self::Scalar(key) => Some(key),
            Self::Variant(index) => Some(index as u128),
            _ => None,
        }
    }
    fn is_total(self, domain: &ir::Ty) -> bool {
        matches!(self, Self::CatchAll)
            || matches!((self, domain), (Self::Scalar(_), ir::Ty::Unit))
            || matches!((self, domain), (Self::Variant(_), ir::Ty::Enum(ty)) if ty.def().variants.len() == 1)
    }
}
enum SourcePatternTest {
    Wildcard,
    Binding,
    Bool(bool),
    Int(ir::IntValue),
    Unit,
    Aggregate(ir::Ty),
    EnumVariant {
        ty: ir::EnumTy,
        variant: usize,
    },
    Decision {
        ty: ir::Ty,
        tests: super::super::token_session::BudgetVec<PendingDecisionTest>,
        coverage: PatternCoverage,
    },
}
struct PendingPatternPlan {
    test: SourcePatternTest,
    bindings: PendingPatternBindings,
}
impl PendingPatternPlan {
    fn insert_coverage(
        &self,
        matrix: &mut super::pattern_usefulness::CoverageMatrix<'_>,
    ) -> Result<bool, String> {
        use super::pattern_usefulness::TestRef;
        match &self.test {
            SourcePatternTest::Decision { tests, .. } => matrix.insert_row(tests.len(), |index| {
                let test = &tests.as_slice()[index];
                TestRef {
                    projection: &test.projection,
                    kind: &test.kind,
                }
            }),
            SourcePatternTest::Bool(value) => {
                let kind = ir::PatternTestKind::Bool(*value);
                matrix.insert_row(1, |_| TestRef {
                    projection: &[],
                    kind: &kind,
                })
            }
            SourcePatternTest::Int(value) => {
                let kind = ir::PatternTestKind::Int(*value);
                matrix.insert_row(1, |_| TestRef {
                    projection: &[],
                    kind: &kind,
                })
            }
            SourcePatternTest::EnumVariant { ty, variant } => {
                let kind = ir::PatternTestKind::EnumVariant {
                    ty: ty.clone(),
                    variant: *variant,
                };
                matrix.insert_row(1, |_| TestRef {
                    projection: &[],
                    kind: &kind,
                })
            }
            _ => matrix.insert_row(0, |_| unreachable!("zero-test coverage row callback")),
        }
    }
    fn coverage(&self) -> PatternCoverage {
        match &self.test {
            SourcePatternTest::Bool(value) => PatternCoverage::Scalar(u128::from(*value)),
            SourcePatternTest::Int(value) => PatternCoverage::Scalar(value.bits),
            SourcePatternTest::Unit => PatternCoverage::Scalar(0),
            SourcePatternTest::EnumVariant { variant, .. } => PatternCoverage::Variant(*variant),
            SourcePatternTest::Decision { coverage, .. } => *coverage,
            _ => PatternCoverage::CatchAll,
        }
    }
}
pub(super) struct PatternLocalInput<'p> {
    key: NameKey,
    name: AstIdent,
    ty: &'p ir::Ty,
    mutable: bool,
    path: &'p [ir::Projection],
    borrow: Option<ir::BorrowKind>,
}
impl PatternLocalInput<'_> {
    fn local_ty(&self) -> ir::Ty {
        match self.borrow {
            None => self.ty.clone(),
            Some(kind) => ir::Ty::Ref {
                kind,
                referent: Arc::new(self.ty.clone()),
            },
        }
    }
}
pub(super) trait PatternLocalSink {
    fn len(&self) -> usize;
    fn push_local(
        &mut self,
        checker: &Checker<'_>,
        input: PatternLocalInput<'_>,
        names: &mut std::collections::HashSet<NameKey>,
    ) -> Result<(), String>;
}
impl PatternLocalSink for Vec<PatternLocal> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn push_local(
        &mut self,
        _checker: &Checker<'_>,
        input: PatternLocalInput<'_>,
        names: &mut std::collections::HashSet<NameKey>,
    ) -> Result<(), String> {
        if !names.insert(input.key) {
            return Err(format!("duplicate pattern binding {:?}", input.name));
        }
        self.push(PatternLocal {
            name: input.name,
            ty: input.local_ty(),
            mutable: input.mutable,
            projection: input.path.to_vec(),
            borrow: input.borrow,
        });
        Ok(())
    }
}
impl PatternLocalSink for PendingPatternBindings {
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn push_local(
        &mut self,
        checker: &Checker<'_>,
        input: PatternLocalInput<'_>,
        _names: &mut std::collections::HashSet<NameKey>,
    ) -> Result<(), String> {
        for row in self.rows.as_slice() {
            checker.infer_work(0)?;
            if row.key == input.key {
                return Err(format!("duplicate pattern binding {:?}", input.name));
            }
        }
        let (projection, lease) = checker.metered_pattern_projection(input.path)?;
        self.rows
            .push(PendingPatternBinding {
                local: PatternLocal {
                    name: input.name,
                    ty: input.local_ty(),
                    mutable: input.mutable,
                    projection,
                    borrow: input.borrow,
                },
                key: input.key,
                _projection_lease: lease,
            })
            .map_err(pattern_token_error)
    }
}
fn pattern_token_error(error: super::super::token_session::TokenError) -> String {
    super::super::source::SourceError::from_token(error).to_string()
}
pub(super) enum SelectedVariantFields<'p> {
    Legacy(Vec<(usize, &'p Pattern)>),
    Positional(super::super::token_session::BudgetVec<(usize, &'p Pattern)>),
}
impl<'p> SelectedVariantFields<'p> {
    fn as_slice(&self) -> &[(usize, &'p Pattern)] {
        match self {
            Self::Legacy(fields) => fields,
            Self::Positional(fields) => fields.as_slice(),
        }
    }
}
pub(super) type ResolvedVariantPattern<'p> = (ir::EnumTy, usize, SelectedVariantFields<'p>);

pub(super) const COUNT_EVAL_WORK: usize = 1_000_000;
#[derive(Clone, Copy, Debug)]
pub(super) enum CountValue {
    Known(ir::IntValue),
    // Keep scalar provenance across cast chains without admitting char arithmetic.
    Char(u32),
    Dynamic,
    Unsupported,
}
#[derive(Clone, Copy)]
pub(super) enum CountMemo {
    Unseen,
    Visiting,
    Done(CountValue),
}
pub(super) struct CountEvaluator<'a> {
    pub(super) exprs: &'a [ir::TypedExpr],
    pub(super) memo: Vec<CountMemo>,
    pub(super) heights: Vec<usize>,
    pub(super) work: usize,
}
impl<'a> CountEvaluator<'a> {
    pub(super) fn new(exprs: &'a [ir::TypedExpr]) -> Result<Self, String> {
        let bytes = exprs
            .len()
            .checked_mul(std::mem::size_of::<CountMemo>() + std::mem::size_of::<usize>())
            .ok_or("shift count memo allocation overflow")?;
        if exprs.len() > COUNT_EVAL_WORK || bytes > 64 * 1024 * 1024 {
            return Err("shift count memo resource limit exceeded".into());
        }
        Ok(Self {
            exprs,
            memo: vec![CountMemo::Unseen; exprs.len()],
            heights: vec![0; exprs.len()],
            work: 0,
        })
    }
    pub(super) fn charge(&mut self) -> Result<(), String> {
        self.work += 1;
        if self.work > COUNT_EVAL_WORK {
            return Err("shift count evaluation work limit exceeded".into());
        }
        Ok(())
    }
    pub(super) fn value(&self, id: ir::ExprId) -> Result<CountValue, String> {
        match self.memo.get(id.0) {
            Some(CountMemo::Done(value)) => Ok(*value),
            _ => Err("invalid shift count evaluation dependency".into()),
        }
    }
    pub(super) fn evaluate(&mut self, root: ir::ExprId) -> Result<CountValue, String> {
        let mut pending = vec![(root, false, 0usize)];
        while let Some((id, ready, depth)) = pending.pop() {
            self.charge()?;
            if depth > 128 {
                return Err("shift count evaluation depth limit exceeded".into());
            }
            let expr = self
                .exprs
                .get(id.0)
                .ok_or("invalid shift count expression ID")?;
            if !ready {
                match self.memo[id.0] {
                    CountMemo::Done(_) => {
                        if depth + self.heights[id.0] > 128 {
                            return Err("shift count evaluation depth limit exceeded".into());
                        }
                        continue;
                    }
                    CountMemo::Visiting => return Err("cycle in shift count expression".into()),
                    CountMemo::Unseen => {}
                }
                // A typed integer that cannot return has no constant value.
                if expr.completion == ir::Completion::Never {
                    self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                    continue;
                }
                let children = match &expr.kind {
                    ir::ExprKind::Int(value) => {
                        value.validate()?;
                        if expr.ty != ir::Ty::Int(value.ty) {
                            return Err("shift count constant type mismatch".into());
                        }
                        self.memo[id.0] = CountMemo::Done(CountValue::Known(*value));
                        continue;
                    }
                    ir::ExprKind::Char(value) => {
                        value.validate()?;
                        if expr.ty != ir::Ty::Char {
                            return Err("shift count character type mismatch".into());
                        }
                        self.memo[id.0] = CountMemo::Done(CountValue::Char(value.value));
                        continue;
                    }
                    ir::ExprKind::Local(_)
                    | ir::ExprKind::Call { .. }
                    | ir::ExprKind::Index { .. }
                    | ir::ExprKind::DynamicIndex { .. }
                    | ir::ExprKind::Deref { .. }
                    | ir::ExprKind::Field { .. } => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                        continue;
                    }
                    ir::ExprKind::Unary { value, .. }
                    | ir::ExprKind::IntCast { value, .. }
                    | ir::ExprKind::LeadingZeros { value }
                    | ir::ExprKind::CharToInt { value, .. }
                    | ir::ExprKind::U8ToChar { value } => [Some(*value), None],
                    ir::ExprKind::PtrCast { .. } => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                        continue;
                    }
                    ir::ExprKind::Binary { left, right, .. } => [Some(*left), Some(*right)],
                    ir::ExprKind::Block(block) if block.statements.is_empty() => [block.tail, None],
                    ir::ExprKind::Block(_) => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                        continue;
                    }
                    _ => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Unsupported);
                        continue;
                    }
                };
                self.memo[id.0] = CountMemo::Visiting;
                pending.push((id, true, depth));
                for child in children.into_iter().flatten().rev() {
                    pending.push((child, false, depth + 1));
                }
                continue;
            }
            let result = match &expr.kind {
                ir::ExprKind::Block(block) => block
                    .tail
                    .map(|id| self.value(id))
                    .transpose()?
                    .unwrap_or(CountValue::Unsupported),
                ir::ExprKind::LeadingZeros { value } => match self.value(*value)? {
                    CountValue::Known(value) => CountValue::Known(ir::IntValue {
                        ty: ir::IntTy::U32,
                        bits: u128::from(value.bits.leading_zeros() - (128 - value.ty.bits())),
                    }),
                    other => other,
                },
                ir::ExprKind::IntCast { value, to } => match self.value(*value)? {
                    CountValue::Known(value) => CountValue::Known(ir::IntValue {
                        ty: *to,
                        bits: count_cast(value, *to),
                    }),
                    other => other,
                },
                ir::ExprKind::CharToInt { value, to } => match self.value(*value)? {
                    CountValue::Char(value) => CountValue::Known(ir::IntValue {
                        ty: *to,
                        bits: u128::from(value) & to.mask(),
                    }),
                    CountValue::Known(_) => {
                        return Err("noncharacter character cast operand".into());
                    }
                    other => other,
                },
                ir::ExprKind::U8ToChar { value } => match self.value(*value)? {
                    CountValue::Known(value) if value.ty == ir::IntTy::U8 => {
                        CountValue::Char(value.bits as u32)
                    }
                    CountValue::Known(_) | CountValue::Char(_) => {
                        return Err("non-u8 character cast operand".into());
                    }
                    other => other,
                },
                ir::ExprKind::Unary { op, value } => match self.value(*value)? {
                    CountValue::Known(value) => {
                        let bits = match op {
                            UnaryOp::Not => !value.bits & value.ty.mask(),
                            UnaryOp::Neg => {
                                if !value.ty.is_signed() {
                                    return Err("unsigned negation in shift count".into());
                                }
                                count_checked_signed(
                                    count_integer(value)
                                        .checked_neg()
                                        .ok_or("arithmetic overflow in static shift count")?,
                                    value.ty,
                                )?
                            }
                        };
                        CountValue::Known(ir::IntValue { ty: value.ty, bits })
                    }
                    other => other,
                },
                ir::ExprKind::Binary { op, left, right } => {
                    let lhs = self.value(*left)?;
                    let rhs = self.value(*right)?;
                    if matches!(op, BinaryOp::Shl | BinaryOp::Shr)
                        && let CountValue::Known(value) = rhs
                    {
                        let ir::Ty::Int(ty) = self.exprs[left.0].ty else {
                            return Err("noninteger nested shift".into());
                        };
                        count_shift_amount(value, ty)?;
                    }
                    match (lhs, rhs) {
                        (CountValue::Known(lhs), CountValue::Known(rhs)) => {
                            count_binary(*op, lhs, rhs)?
                        }
                        (CountValue::Unsupported, _) | (_, CountValue::Unsupported) => {
                            CountValue::Unsupported
                        }
                        _ => CountValue::Dynamic,
                    }
                }
                _ => return Err("invalid shift count evaluation state".into()),
            };
            if let CountValue::Known(value) = result
                && expr.ty != ir::Ty::Int(value.ty)
            {
                return Err("shift count result type mismatch".into());
            }
            if matches!(result, CountValue::Char(_)) && expr.ty != ir::Ty::Char {
                return Err("shift count character result type mismatch".into());
            }
            let height = match &expr.kind {
                ir::ExprKind::Block(block) => {
                    block.tail.map(|id| self.heights[id.0] + 1).unwrap_or(0)
                }
                ir::ExprKind::IntCast { value, .. }
                | ir::ExprKind::Unary { value, .. }
                | ir::ExprKind::LeadingZeros { value }
                | ir::ExprKind::CharToInt { value, .. }
                | ir::ExprKind::U8ToChar { value }
                | ir::ExprKind::PtrCast { value } => self.heights[value.0] + 1,
                ir::ExprKind::Binary { left, right, .. } => {
                    self.heights[left.0].max(self.heights[right.0]) + 1
                }
                _ => 0,
            };
            if depth + height > 128 {
                return Err("shift count evaluation depth limit exceeded".into());
            }
            self.heights[id.0] = height;
            self.memo[id.0] = CountMemo::Done(result);
        }
        self.value(root)
    }
}
pub(super) fn count_integer(value: ir::IntValue) -> i128 {
    // Sign extension by shifts avoids constructing 2^128 or negating MIN.
    let shift = 128 - value.ty.bits();
    ((value.bits << shift) as i128) >> shift
}
pub(super) fn count_cast(value: ir::IntValue, to: ir::IntTy) -> u128 {
    let extended = if value.ty.is_signed() {
        count_integer(value) as u128
    } else {
        value.bits
    };
    extended & to.mask()
}
pub(super) fn count_checked_signed(value: i128, ty: ir::IntTy) -> Result<u128, String> {
    if ty.bits() < 128 {
        let limit = 1i128 << (ty.bits() - 1);
        if value < -limit || value >= limit {
            return Err("arithmetic overflow in static shift count".into());
        }
    }
    Ok(value as u128 & ty.mask())
}
pub(super) fn count_shift_amount(value: ir::IntValue, lhs: ir::IntTy) -> Result<u32, String> {
    value.validate()?;
    if (value.ty.is_signed() && count_integer(value) < 0) || value.bits >= u128::from(lhs.bits()) {
        let amount = if value.ty.is_signed() {
            count_integer(value).to_string()
        } else {
            value.bits.to_string()
        };
        return Err(format!(
            "shift count {amount} overflows {}-bit integer",
            lhs.bits()
        ));
    }
    Ok(value.bits as u32)
}
pub(super) fn count_binary(
    op: BinaryOp,
    lhs: ir::IntValue,
    rhs: ir::IntValue,
) -> Result<CountValue, String> {
    lhs.validate()?;
    rhs.validate()?;
    let ty = lhs.ty;
    let bits = if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
        let amount = count_shift_amount(rhs, ty)?;
        match op {
            BinaryOp::Shl => (lhs.bits << amount) & ty.mask(),
            _ if ty.is_signed() => (count_integer(lhs) >> amount) as u128 & ty.mask(),
            _ => lhs.bits >> amount,
        }
    } else {
        if lhs.ty != rhs.ty {
            return Err("static shift count integer type mismatch".into());
        }
        match op {
            BinaryOp::BitAnd => lhs.bits & rhs.bits,
            BinaryOp::BitOr => lhs.bits | rhs.bits,
            BinaryOp::BitXor => lhs.bits ^ rhs.bits,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                if ty.is_signed() {
                    let a = count_integer(lhs);
                    let b = count_integer(rhs);
                    if matches!(op, BinaryOp::Div | BinaryOp::Rem)
                        && (b == 0
                            || (a
                                == count_integer(ir::IntValue {
                                    ty,
                                    bits: 1u128 << (ty.bits() - 1),
                                })
                                && b == -1))
                    {
                        return Err("invalid division in static shift count".into());
                    }
                    let value = match op {
                        BinaryOp::Add => a.checked_add(b),
                        BinaryOp::Sub => a.checked_sub(b),
                        BinaryOp::Mul => a.checked_mul(b),
                        BinaryOp::Div => a.checked_div(b),
                        _ => a.checked_rem(b),
                    }
                    .ok_or("arithmetic overflow in static shift count")?;
                    count_checked_signed(value, ty)?
                } else {
                    let a = lhs.bits;
                    let b = rhs.bits;
                    match op {
                        BinaryOp::Add => a.checked_add(b),
                        BinaryOp::Sub => a.checked_sub(b),
                        BinaryOp::Mul => a.checked_mul(b),
                        BinaryOp::Div => a.checked_div(b),
                        _ => a.checked_rem(b),
                    }
                    .filter(|v| *v <= ty.mask())
                    .ok_or("invalid arithmetic in static shift count")?
                }
            }
            _ => return Ok(CountValue::Unsupported),
        }
    };
    Ok(CountValue::Known(ir::IntValue { ty, bits }))
}
pub(super) fn validate_shift_counts(
    exprs: &[ir::TypedExpr],
    queries: &[(ir::IntTy, ir::ExprId)],
) -> Result<(), String> {
    if queries.is_empty()
        && !exprs.iter().any(|e| {
            matches!(
                e.kind,
                ir::ExprKind::Binary {
                    op: BinaryOp::Shl | BinaryOp::Shr,
                    ..
                }
            )
        })
    {
        return Ok(());
    }
    let mut evaluator = CountEvaluator::new(exprs)?;
    if queries.len() > COUNT_EVAL_WORK {
        return Err("compound shift query work limit exceeded".into());
    }
    for &(ty, value) in queries {
        evaluator.charge()?;
        match evaluator.evaluate(value)? {
            CountValue::Known(value) => {
                count_shift_amount(value, ty)?;
            }
            CountValue::Dynamic => {}
            CountValue::Unsupported | CountValue::Char(_) => {
                return Err("unsupported static-shaped compound shift count expression".into());
            }
        }
    }
    for expr in exprs {
        evaluator.charge()?;
        if let ir::ExprKind::Binary {
            op: BinaryOp::Shl | BinaryOp::Shr,
            left,
            right,
        } = expr.kind
        {
            let ir::Ty::Int(lhs) = exprs.get(left.0).ok_or("invalid shift lhs")?.ty else {
                return Err("noninteger shift lhs".into());
            };
            match evaluator.evaluate(right)? {
                CountValue::Known(value) => {
                    count_shift_amount(value, lhs)?;
                }
                CountValue::Dynamic => {}
                CountValue::Unsupported | CountValue::Char(_) => {
                    return Err("unsupported static-shaped shift count expression".into());
                }
            }
        }
    }
    Ok(())
}
// The legacy adapter shares all constraints with source replay, but keeps its admission fence.
use super::super::source_replay::{CallInferenceState, InferKind, InferTerm};
pub(super) enum InferField<'a> {
    Named(&'a AstIdent),
    Tuple(usize),
}
pub(super) struct Inference<'c, 's> {
    pub(super) checker: &'c Checker<'s>,
    pub(super) core: CallInferenceState,
}
impl<'c, 's> Inference<'c, 's> {
    pub(super) fn new(checker: &'c Checker<'s>, env: Arc<ir::ParamEnv>) -> Result<Self, String> {
        let budget = &checker
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?
            .budget;
        let core = CallInferenceState::new(Some(env), budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        Ok(Self { checker, core })
    }
    pub(super) fn empty(checker: &'c Checker<'s>) -> Result<Self, String> {
        let budget = &checker
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?
            .budget;
        let core =
            CallInferenceState::new(None, budget).map_err(|e| e.into_legacy_diagnostic(budget))?;
        Ok(Self { checker, core })
    }
    pub(super) fn schema(&self, ty: &ir::Ty, depth: usize) -> Result<InferTerm, String> {
        self.core.schema(ty, depth).map_err(|e| {
            e.into_legacy_diagnostic(&self.checker.signatures.plan.as_ref().unwrap().budget)
        })
    }
    pub(super) fn merge(
        &self,
        a: &InferTerm,
        b: &InferTerm,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.core.merge(a, b, depth).map_err(|e| {
            e.into_legacy_diagnostic(&self.checker.signatures.plan.as_ref().unwrap().budget)
        })
    }
    pub(super) fn constrain(
        &mut self,
        a: &InferTerm,
        b: &InferTerm,
        depth: usize,
    ) -> Result<(), String> {
        self.core.constrain(a, b, depth).map_err(|e| {
            e.into_legacy_diagnostic(&self.checker.signatures.plan.as_ref().unwrap().budget)
        })
    }
    pub(super) fn materialize(
        &self,
        declaration: ir::DeclId,
        actuals: &[ir::Ty],
    ) -> Result<ir::Ty, FactsError> {
        let plan = self.checker.signatures.plan.as_ref().unwrap();
        if let Some(inputs) = self.checker.nominal_inputs {
            if nominal_inputs::parent_local::contains_closure(actuals, &plan.budget, 0)
                .map_err(|e| source_error(&plan.budget, &e))?
            {
                return self
                    .checker
                    .mint_parent_local_nominal(declaration, actuals)
                    .map_err(|e| source_error(&plan.budget, &e));
            }
            let demand = self
                .checker
                .nominal_input_demand(declaration, actuals)
                .map_err(|e| source_error(&plan.budget, &e))?;
            return (inputs.nominal_type)(&demand)
                .map_err(|e| source_error(&plan.budget, &e))?
                .ok_or_else(|| {
                    source_error(
                        &plan.budget,
                        "missing graph input: inferred nominal realization",
                    )
                });
        }
        plan.intern_nominal(declaration, actuals, self.checker.facts(), &plan.budget)
            .map_err(|e| source_error(&plan.budget, &e))
    }
    pub(super) fn resolve(
        &self,
        term: &InferTerm,
        fallback: bool,
        depth: usize,
    ) -> Result<Option<ir::Ty>, String> {
        let budget = &self.checker.signatures.plan.as_ref().unwrap().budget;
        self.core
            .resolve(term, fallback, depth, &mut |decl, args| {
                self.materialize(decl, args)
            })
            .map_err(|e| e.into_legacy_diagnostic(budget))
    }
    pub(super) fn finish(&self, fallback: bool) -> Result<Option<Vec<ir::Ty>>, String> {
        let plan = self.checker.signatures.plan.as_ref().unwrap();
        let Some(args) = self
            .core
            .finish(fallback, &mut |decl, args| self.materialize(decl, args))
            .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?
        else {
            return Ok(None);
        };
        for ty in &args {
            // Only the canonical (replayed) checker carries inferred reference
            // actuals with origins. Legacy cloned-Program adapters cannot.
            if self.checker.canonical.is_none()
                && contains_source_ref(ty, &plan.budget, 0)
                    .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?
            {
                return Err("unsupported generic inference: newly inferred reference actual requires the canonical source checker".into());
            }
            validate_argument(ty, self.checker.facts(), &plan.budget, 0)?;
        }
        Ok(Some(args))
    }
}
impl<'s> Checker<'s> {
    pub(super) fn inference_core(&self) -> Result<CallInferenceState, String> {
        let budget = &self
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?
            .budget;
        CallInferenceState::new(None, budget).map_err(|e| e.into_legacy_diagnostic(budget))
    }
    pub(super) fn infer_work(&self, depth: usize) -> Result<(), String> {
        self.inference_core()?
            .work(depth)
            .map_err(|e| e.into_legacy_diagnostic(&self.signatures.plan.as_ref().unwrap().budget))
    }
    pub(super) fn infer_storage(&self, count: usize) -> Result<(), String> {
        self.inference_core()?
            .storage(count)
            .map_err(|e| e.into_legacy_diagnostic(&self.signatures.plan.as_ref().unwrap().budget))
    }
    pub(super) fn infer_node(&self, kind: InferKind) -> Result<InferTerm, String> {
        self.inference_core()?
            .node(kind)
            .map_err(|e| e.into_legacy_diagnostic(&self.signatures.plan.as_ref().unwrap().budget))
    }
    pub(super) fn infer_type_term(
        &self,
        ty: &ir::Ty,
        schema: Option<ir::BinderId>,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.inference_core()?
            .type_term(ty, schema, depth)
            .map_err(|e| e.into_legacy_diagnostic(&self.signatures.plan.as_ref().unwrap().budget))
    }
    pub(super) fn inference_evidence(
        &self,
        expr: &Expr,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        // Local IDs grow monotonically, including shadowing. Scope depth distinguishes pops.
        let key = (
            expr as *const Expr as usize,
            self.locals.len(),
            self.scopes.len(),
        );
        if let Some(term) = self.inference_evidence_cache.borrow().get(&key) {
            return Ok(term.clone());
        }
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?;
        let term = match expr {
            Expr::NativeSyscall(_) | Expr::TypeAlignment(_) => {
                self.infer_node(InferKind::Rigid(ir::Ty::Int(ir::IntTy::Usize)))?
            }
            Expr::RawSlice { kind, arguments } => match self.raw_slice_hint(*kind, arguments)? {
                Some(ty) => self.infer_type_term(&ty, None, depth + 1)?,
                None => self.infer_node(InferKind::Unknown(false))?,
            },
            Expr::UnexpandedMacro(_) => return Err(UNEXPANDED_MACRO_CHECK_ERROR.into()),
            Expr::Output(output) => {
                validate_output_site(output, self.names()?)?;
                self.infer_node(InferKind::Rigid(ir::Ty::Unit))?
            }
            Expr::Panic(panic) => {
                validate_panic_site(panic, self.names()?)?;
                self.infer_node(InferKind::Rigid(ir::Ty::Never))?
            }
            Expr::Integer(literal) => self.infer_node(match literal.suffix {
                Some(t) => InferKind::Rigid(ir::Ty::Int(t)),
                None => InferKind::Integer,
            })?,
            Expr::Float(literal) => self.infer_node(match literal.suffix {
                Some(t) => InferKind::Rigid(ir::Ty::Float(t)),
                None => InferKind::Float,
            })?,
            Expr::Bool(_) => self.infer_node(InferKind::Rigid(ir::Ty::Bool))?,
            Expr::Char(_) => self.infer_node(InferKind::Rigid(ir::Ty::Char))?,
            Expr::Str(_) => self.infer_node(InferKind::Rigid(ir::Ty::shared_str_ty()))?,
            Expr::ByteStr(bytes) => {
                self.infer_node(InferKind::Rigid(ir::Ty::byte_string_ty(bytes.len())))?
            }
            Expr::ConstItem(_) | Expr::AssociatedConst(_) => {
                let ty = self
                    .canonical
                    .ok_or("contract: constant inference requires sealed graph inputs")?
                    .constant_type(expr)?;
                self.infer_node(InferKind::Rigid(ty))?
            }
            Expr::Var(name) => match self.callable_value_hint(name)? {
                Some(ty) => self.infer_type_term(&ty, None, depth + 1)?,
                None => self.infer_node(InferKind::Unknown(false))?,
            },
            Expr::Borrow { kind, place } => {
                plan.budget.charge_work(place.fields.len() + 1)?;
                self.infer_storage(place.fields.len() + 1)?;
                if let Some(ir::Ty::Ref { referent, .. }) = self.static_index_borrow_hint(place)? {
                    let inner = self.infer_type_term(&referent, None, depth + 1)?;
                    return self.infer_node(InferKind::Ref(*kind, inner));
                }
                let (_, referent, _) = self.checked_place(place)?;
                self.validate_borrow_referent(*kind, &referent)?;
                let inner = self.infer_type_term(&referent, None, depth + 1)?;
                self.infer_node(InferKind::Ref(*kind, inner))?
            }
            Expr::BorrowValue { kind, value } => {
                let inner = self.inference_evidence(value, depth + 1)?;
                self.infer_node(InferKind::Ref(*kind, inner))?
            }
            Expr::Deref { value } => {
                let carrier = self.inference_evidence(value, depth + 1)?;
                match carrier.as_ref() {
                    InferKind::Ref(_, inner) | InferKind::RawPtr(_, inner) => inner.clone(),
                    _ => self.infer_node(InferKind::Unknown(false))?,
                }
            }
            Expr::Tuple(fields) => {
                self.infer_storage(fields.len())?;
                self.infer_node(InferKind::Tuple(
                    fields
                        .iter()
                        .map(|f| self.inference_evidence(f, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                ))?
            }
            Expr::Array(fields) => {
                let mut elements = None;
                let dummy = Inference::empty(self)?;
                for f in fields {
                    let term = self.inference_evidence(f, depth + 1)?;
                    elements = Some(if let Some(old) = elements {
                        dummy.merge(&old, &term, depth + 1)?
                    } else {
                        term
                    });
                }
                self.infer_node(InferKind::Array(
                    elements.unwrap_or(self.infer_node(InferKind::Unknown(false))?),
                    fields.len(),
                ))?
            }
            Expr::Repeat { value, len } => self.infer_node(InferKind::Array(
                self.inference_evidence(value, depth + 1)?,
                *len,
            ))?,
            Expr::Unary { value, .. }
                if matches!(value.as_ref(), Expr::Integer(_) | Expr::Float(_)) =>
            {
                self.inference_evidence(value, depth + 1)?
            }
            Expr::Block(block) if block.statements.is_empty() && block.tail.is_none() => {
                self.infer_node(InferKind::Rigid(ir::Ty::Unit))?
            }
            Expr::Cast { target, .. } => {
                self.infer_type_term(&checked_local_ty(target, self.signatures)?, None, depth + 1)?
            }
            Expr::Call { name, .. } => {
                let declaration = *plan
                    .registry
                    .functions
                    .get(&self.path_key(name, Namespace::Value)?)
                    .ok_or("unresolved function declaration in inference")?;
                let schema = plan.function_schema(declaration)?;
                if !schema.env.is_generic() {
                    self.infer_type_term(&schema.signature.result, None, depth + 1)?
                } else {
                    self.infer_node(InferKind::Unknown(false))?
                }
            }
            Expr::Struct { name, .. } if self.self_path(name)? => self.infer_type_term(
                self.signatures
                    .self_ty
                    .as_ref()
                    .ok_or("Self outside impl")?,
                None,
                depth + 1,
            )?,
            Expr::TraitCall { .. } | Expr::AssociatedCall { .. } | Expr::MethodCall { .. } => {
                if let Some(ty) = self.scalar_hint(expr)? {
                    self.infer_type_term(&ty, None, depth + 1)?
                } else {
                    self.infer_node(InferKind::Unknown(false))?
                }
            }
            Expr::Struct { name, .. }
            | Expr::TupleStruct { name, .. }
            | Expr::EnumVariant { name, .. } => {
                let owner = if matches!(expr, Expr::EnumVariant { .. }) {
                    name.semantic_parent(&*plan.program.names)
                        .map_err(native_adapter::name_diagnostic)?
                } else {
                    name.clone()
                };
                let decl = *plan
                    .registry
                    .nominals
                    .get(&self.path_key(&owner, Namespace::Type)?)
                    .ok_or("unresolved nominal declaration in inference")?;
                let (env, ty) = plan.nominal_schema(decl, &plan.budget)?;
                if env.arity() == 0 {
                    self.infer_type_term(&ty, None, depth + 1)?
                } else {
                    let count = match expr {
                        Expr::Struct { fields, .. } => fields.len(),
                        Expr::TupleStruct { fields, .. } => fields.len(),
                        Expr::EnumVariant { fields, .. } => fields.len(),
                        _ => return Err("constructor evidence dispatch invariant".into()),
                    };
                    self.infer_storage(count)?;
                    let mut evidence = self.retained_pattern_vec(count)?;
                    let variant = match expr {
                        Expr::Struct { fields, .. } => {
                            for (name, value) in fields {
                                evidence.push((InferField::Named(name), value));
                            }
                            None
                        }
                        Expr::TupleStruct { fields, .. } => {
                            for (index, value) in fields.iter().enumerate() {
                                evidence.push((InferField::Tuple(index), value));
                            }
                            None
                        }
                        Expr::EnumVariant { fields, .. } => {
                            for (selector, value) in fields {
                                let selector = match selector {
                                    FieldSelector::Tuple(index) => InferField::Tuple(*index),
                                    FieldSelector::Named(name) => InferField::Named(name),
                                    _ => return Err("invalid constructor evidence selector".into()),
                                };
                                evidence.push((selector, value));
                            }
                            Some(self.split_variant(name)?.1)
                        }
                        _ => return Err("constructor evidence dispatch invariant".into()),
                    };
                    self.constructor_evidence(&owner, variant, &evidence, depth + 1)?
                }
            }
            Expr::Apply { path, value } => {
                let (_, args) = if let Some(application) =
                    self.partial_call_application(path, value, None, true)?
                {
                    application
                } else {
                    plan.application(
                        path,
                        application_namespace(value)?,
                        &self.signatures.params,
                        self.facts(),
                        &plan.budget,
                    )?
                };
                let ty = match value.as_ref() {
                    Expr::Call { name, .. } => {
                        let declaration = *plan
                            .registry
                            .functions
                            .get(&self.path_key(name, Namespace::Value)?)
                            .ok_or("unresolved function declaration in inference")?;
                        plan.registry
                            .check_explicit_impl_params(declaration, path.application()?.1.len())?;
                        plan.instantiate_signature(
                            &ir::CallTarget {
                                declaration,
                                type_args: args.into(),
                            },
                            self.facts(),
                            &plan.budget,
                        )?
                        .result
                    }
                    Expr::Struct { name, .. } | Expr::TupleStruct { name, .. } => {
                        let decl = *plan
                            .registry
                            .nominals
                            .get(&self.path_key(name, Namespace::Type)?)
                            .ok_or("unresolved nominal declaration in inference")?;
                        plan.intern_nominal(decl, &args, self.facts(), &plan.budget)?
                    }
                    Expr::EnumVariant { name, .. } => {
                        let (owner, _) = self.split_variant(name)?;
                        let decl = *plan
                            .registry
                            .nominals
                            .get(&self.path_key(&owner, Namespace::Type)?)
                            .ok_or("unresolved enum declaration in inference")?;
                        plan.intern_nominal(decl, &args, self.facts(), &plan.budget)?
                    }
                    _ => return Err("unsupported explicit generic evidence form".into()),
                };
                self.infer_type_term(&ty, None, depth + 1)?
            }
            Expr::Field { base, field } => {
                let base = self.inference_evidence(base, depth + 1)?;
                let dummy = Inference::empty(self)?;
                if let Some(ty) = dummy.resolve(&base, false, depth + 1)? {
                    let receiver = if let ir::Ty::Ref { referent, .. } = &ty {
                        referent.as_ref()
                    } else {
                        &ty
                    };
                    let (_, ty) = self.field_type(receiver, field)?;
                    self.infer_type_term(&ty, None, depth + 1)?
                } else {
                    self.infer_node(InferKind::Unknown(false))?
                }
            }
            Expr::Index { base, index, .. } => {
                match self.inference_evidence(base, depth + 1)?.as_ref() {
                    InferKind::Array(element, len) if index < len => element.clone(),
                    InferKind::Array(_, _) => return Err("array index out of bounds".into()),
                    _ => self.infer_node(InferKind::Unknown(false))?,
                }
            }
            Expr::If {
                then_block,
                else_block,
                ..
            } => {
                // Only statement-free branches: a tail could otherwise name
                // a block-local that shadows an outer binding.
                let mut term = None;
                for block in [then_block, else_block] {
                    if let (true, Some(tail)) = (block.statements.is_empty(), &block.tail) {
                        let t = self.inference_evidence(tail, depth + 1)?;
                        if !matches!(t.as_ref(), InferKind::Unknown(_)) {
                            term = Some(t);
                            break;
                        }
                    }
                }
                match term {
                    Some(t) => t,
                    None => self.infer_node(InferKind::Unknown(false))?,
                }
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                use BinaryOp::*;
                match op {
                    Eq | Ne | Lt | Le | Gt | Ge | And | Or => {
                        self.infer_node(InferKind::Rigid(ir::Ty::Bool))?
                    }
                    _ => {
                        // Builtin arithmetic: the result is the operand type
                        // when that is a primitive scalar (or literal kind).
                        // Overloaded operators on other types give no evidence.
                        let primitive = |t: &InferTerm| {
                            matches!(
                                t.as_ref(),
                                InferKind::Integer
                                    | InferKind::Float
                                    | InferKind::Rigid(
                                        ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool
                                    )
                            )
                        };
                        let l = self.inference_evidence(left, depth + 1)?;
                        if matches!(op, Shl | Shr) {
                            if primitive(&l) {
                                l
                            } else {
                                self.infer_node(InferKind::Unknown(false))?
                            }
                        } else {
                            let r = self.inference_evidence(right, depth + 1)?;
                            let rigid = |t: &InferTerm| matches!(t.as_ref(), InferKind::Rigid(_));
                            if primitive(&l) && primitive(&r) {
                                if rigid(&l) || !rigid(&r) { l } else { r }
                            } else {
                                self.infer_node(InferKind::Unknown(false))?
                            }
                        }
                    }
                }
            }
            _ => self.infer_node(InferKind::Unknown(false))?,
        };
        self.infer_storage(1)?;
        self.inference_evidence_cache
            .borrow_mut()
            .insert(key, term.clone());
        Ok(term)
    }
    pub(super) fn infer_call_args(
        &self,
        declaration: ir::DeclId,
        args: &[Expr],
        expected: Option<&ir::Ty>,
        fallback: bool,
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        self.infer_call_args_with_partial(declaration, args, expected, fallback, None)
    }
    pub(super) fn infer_call_args_with_partial(
        &self,
        declaration: ir::DeclId,
        args: &[Expr],
        expected: Option<&ir::Ty>,
        fallback: bool,
        explicit: Option<&[Type]>,
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?;
        if explicit.is_none()
            && let Some(cap) = self.canonical
            && let Some(actual) = cap.shared_actual(declaration, args, expected, self)?
        {
            self.infer_storage(1)?;
            let mut actuals = Vec::new();
            actuals.try_reserve_exact(1).map_err(|_| {
                cap.render(FactsError::resource(
                    &plan.budget,
                    "shared actual allocation",
                ))
            })?;
            actuals.push(actual);
            return Ok(Some(actuals));
        }
        let schema = plan.function_schema(declaration)?;
        if schema.signature.params.len() != args.len() {
            return Err("generic function argument count mismatch".into());
        }
        let mut solver = Inference::new(self, schema.env.scopes()[0].clone())?;
        if let Some(explicit) = explicit {
            let anonymous = plan
                .registry
                .source_callable(declaration)
                .map_or(0, |f| f.impl_params);
            if explicit.len() + anonymous != schema.env.scopes()[0].arity() {
                return Err("E0107: wrong number of generic arguments".into());
            }
            for (slot, ty) in explicit.iter().enumerate() {
                let formal = self.infer_node(InferKind::Slot(slot))?;
                let actual = self.partial_type_term(ty, 0)?;
                solver.constrain(&formal, &actual, 0)?;
            }
        }
        let mut projected_constraints = Vec::new();
        if let Some(expected) = expected {
            let evidence = self.infer_type_term(expected, None, 0)?;
            projected_constraints.push((&schema.signature.result, evidence));
        }
        for (formal, expr) in schema.signature.params.iter().zip(args) {
            let evidence = self.inference_evidence(expr, 0)?;
            if let Some((kind, element)) = formal.slice_ref_element()
                && let InferKind::Ref(actual_kind, referent) = evidence.as_ref()
                && kind == *actual_kind
            {
                self.infer_work(1)?;
                let actual = match referent.as_ref() {
                    InferKind::Array(element, _) | InferKind::Slice(element) => {
                        Some(element.clone())
                    }
                    InferKind::Rigid(ir::Ty::Slice { element }) => {
                        Some(self.infer_type_term(element, None, 1)?)
                    }
                    _ => None,
                };
                if let Some(actual) = actual {
                    projected_constraints.push((element, actual));
                    continue;
                }
            }
            projected_constraints.push((formal, evidence));
        }
        // A declared `Proj = Rhs` bound is evidence for slots in `Rhs` once
        // the projection's inputs are solved: normalize the projection and
        // unify it with the schema of `Rhs` (e.g. `C: Tr<Item = Vec<T>>`).
        let equalities: Vec<(ir::Ty, &ir::Ty)> = schema
            .predicates
            .predicates()
            .iter()
            .filter_map(|predicate| match predicate {
                // Bare-parameter RHS is already covered by projected formals.
                ir::Predicate::ProjectionEq { projection, ty }
                    if !matches!(ty, ir::Ty::Param(_)) && inferred_owner::mentions_param(ty) =>
                {
                    Some((ir::Ty::AssociatedType(Arc::new(projection.clone())), ty))
                }
                _ => None,
            })
            .collect();
        self.infer_storage(equalities.len())?;
        if !solver.constrain_projected_formals(projected_constraints, fallback)? {
            return Ok(None);
        }
        // Equalities whose projection inputs stay unsolved add no evidence;
        // the obligation itself is still checked against the final actuals.
        for (projection, rhs) in &equalities {
            solver.constrain_projection_equality(projection, rhs, fallback)?;
        }
        solver.finish(fallback)
    }
    fn constructor_constraints<'c>(
        &'c self,
        name: &AstPath,
        variant: Option<AstIdent>,
        fields: &[(InferField<'_>, &Expr)],
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<(ir::DeclId, Inference<'c, 's>), String> {
        self.infer_work(depth)?;
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("inference requires planning context")?;
        let declaration = *plan
            .registry
            .nominals
            .get(&self.path_key(name, Namespace::Type)?)
            .ok_or("unknown nominal declaration")?;
        let (env, schema) = plan.nominal_schema(declaration, &plan.budget)?;
        let mut solver = Inference::new(self, env)?;
        if let Some(expected) = expected {
            let formal = solver.schema(&schema, depth + 1)?;
            let evidence = self.infer_type_term(expected, None, depth + 1)?;
            solver.constrain(&formal, &evidence, depth + 1)?;
        }
        let variant_index = variant
            .map(|name| self.variant_index(declaration, name))
            .transpose()?;
        // Keep declaration/schema authority above. Only the field view changes:
        // a closed expected application has already read its normalized graph
        // definition, rather than a rigid projection from the raw schema.
        let realized = self.realized_constructor_expectation(declaration, expected)?;
        let field_source = realized.as_ref().unwrap_or(&schema);
        let definitions = match (field_source, variant_index) {
            (ir::Ty::Struct(ty), None) => &ty.def().fields,
            (ir::Ty::Enum(ty), Some(index)) => {
                &ty.def()
                    .variants
                    .get(index)
                    .ok_or("unknown enum variant")?
                    .fields
            }
            _ => return Err("constructor declaration kind mismatch".into()),
        };
        for (selector, expr) in fields {
            let index = match selector {
                InferField::Tuple(index) => *index,
                InferField::Named(name) => self.member_index(declaration, variant_index, **name)?,
            };
            let field = definitions.get(index).ok_or("unknown constructor field")?;
            let formal = solver.schema(&field.ty, depth + 1)?;
            let evidence = self.inference_evidence(expr, depth + 1)?;
            solver.constrain(&formal, &evidence, depth + 1)?;
        }
        Ok((declaration, solver))
    }
    fn check_slot_free_evidence(&self, term: &InferTerm, depth: usize) -> Result<(), String> {
        self.infer_work(depth)?;
        match term.as_ref() {
            InferKind::Slot(_) => {
                return Err("contract: constructor schema slot escaped into evidence".into());
            }
            InferKind::Tuple(fields)
            | InferKind::Nominal(_, fields)
            | InferKind::Opaque(_, fields) => {
                for field in fields.iter() {
                    self.check_slot_free_evidence(field, depth + 1)?;
                }
            }
            InferKind::Array(inner, _)
            | InferKind::Slice(inner)
            | InferKind::Ref(_, inner)
            | InferKind::RawPtr(_, inner) => self.check_slot_free_evidence(inner, depth + 1)?,
            _ => {}
        }
        Ok(())
    }
    fn constructor_evidence(
        &self,
        name: &AstPath,
        variant: Option<AstIdent>,
        fields: &[(InferField<'_>, &Expr)],
        depth: usize,
    ) -> Result<InferTerm, String> {
        let (declaration, solver) =
            self.constructor_constraints(name, variant, fields, None, depth + 1)?;
        let slots = solver.core.slots();
        self.infer_storage(slots.len())?;
        let mut terms = self.retained_pattern_vec(slots.len())?;
        for slot in slots {
            self.infer_work(depth + 1)?;
            let term = match slot {
                Some(term) => term.clone(),
                None => self.infer_node(InferKind::Unknown(false))?,
            };
            self.check_slot_free_evidence(&term, depth + 1)?;
            terms.push(term);
        }
        // Preserve literal classes until the OUTER solver's existing fallback.
        self.infer_node(InferKind::Nominal(declaration, terms.into()))
    }
    /// Joint type evidence of two statement-free branch tails, used only when
    /// an `if` has no outer expectation. None leaves checking unchanged.
    pub(super) fn branch_evidence_type(
        &self,
        then_block: &Block,
        else_block: &Block,
    ) -> Result<Option<ir::Ty>, String> {
        fn tail(b: &Block) -> Option<&Expr> {
            if b.statements.is_empty() {
                b.tail.as_deref()
            } else {
                None
            }
        }
        let (Some(a), Some(b)) = (tail(then_block), tail(else_block)) else {
            return Ok(None);
        };
        let constructor = |e: &Expr| {
            matches!(
                e,
                Expr::Call { .. }
                    | Expr::EnumVariant { .. }
                    | Expr::TupleStruct { .. }
                    | Expr::Var(_)
            )
        };
        if !constructor(a) || !constructor(b) {
            return Ok(None);
        }
        let a = self.inference_evidence(a, 0)?;
        let b = self.inference_evidence(b, 0)?;
        let mut solver = Inference::empty(self)?;
        if solver.constrain(&a, &b, 0).is_err() {
            return Ok(None);
        }
        Ok(solver.resolve(&a, true, 0).ok().flatten())
    }
    pub(super) fn infer_constructor(
        &self,
        name: &AstPath,
        variant: Option<AstIdent>,
        fields: &[(InferField<'_>, &Expr)],
        expected: Option<&ir::Ty>,
        fallback: bool,
    ) -> Result<Option<ir::Ty>, String> {
        let (declaration, solver) =
            self.constructor_constraints(name, variant, fields, expected, 0)?;
        let Some(args) = solver.finish(fallback)? else {
            return Ok(None);
        };
        Ok(Some(self.materialize_nominal_input(declaration, &args)?))
    }
}
pub(super) fn genuine_call(expr: &Expr) -> bool {
    match expr {
        Expr::Call { .. }
        | Expr::AssociatedCall { .. }
        | Expr::TraitCall { .. }
        | Expr::MethodCall { .. } => true,
        Expr::Apply { value, .. } => genuine_call(value),
        _ => false,
    }
}
/// Place expressions that an rvalue temporary would wrongly copy.
pub(super) fn place_expression(expr: &Expr) -> bool {
    match expr {
        Expr::Var(_) | Expr::Deref { .. } | Expr::DynamicIndex { .. } => true,
        Expr::Field { base, .. } | Expr::Index { base, .. } => place_expression(base),
        _ => false,
    }
}
pub(super) fn static_source_place(mut expr: &Expr) -> Result<Place, String> {
    let mut fields = Vec::new();
    loop {
        if fields.len() >= 128 {
            return Err("receiver place nesting limit128".into());
        }
        match expr {
            Expr::UnexpandedMacro(_) => return Err(UNEXPANDED_MACRO_CHECK_ERROR.into()),
            Expr::Var(local) => {
                fields.reverse();
                return Ok(Place {
                    local: match (local.anchor(), local.segments()) {
                        (PathAnchor::SelfModule, []) => receiver_authority::local(local),
                        (PathAnchor::Relative, [name]) => LocalName::Named(*name),
                        _ => return Err("receiver requires a local-root path".into()),
                    },
                    fields,
                });
            }
            Expr::Field { base, field } => {
                fields.push(field.clone());
                expr = base;
            }
            Expr::Index { base, index, .. } => {
                fields.push(FieldSelector::ConstantIndex(*index));
                expr = base;
            }
            Expr::Deref { value } => {
                fields.push(FieldSelector::Deref);
                expr = value;
            }
            _ => {
                return Err(
                    "owned rvalue or dynamic borrowed receiver places are unsupported".into(),
                );
            }
        }
    }
}
pub(super) struct PendingReferenceDecl {
    pub(super) name: AstIdent,
    pub(super) mutable: bool,
    pub(super) scope: usize,
    pub(super) position: usize,
    pub(super) local: Option<ir::LocalId>,
}
pub(super) type CheckedApplication = (PathKey, Vec<ir::Ty>, Arc<[ir::TypeActualRegion]>);

pub(super) struct Checker<'a> {
    pub(super) runtime_sites: Vec<(ir::ExprId, Arc<ir::RuntimeLocation>)>,
    pub(super) signatures: &'a Signatures,
    pub(super) canonical: Option<&'a CanonicalSourceCapability>,
    // Borrowed only within this checking node. Child callable owners do not
    // inherit this capability adapter implicitly. Closure bodies are part of
    // the same checking node and share it (closures/prepare.rs).
    pub(in crate::compiler::frontend) nominal_inputs: Option<&'a NominalPreparationInputs<'a>>,
    // Local suspension inventory propagated to the owning graph consumer.
    pub(in crate::compiler::frontend) nominal_blocked:
        std::cell::RefCell<Option<NominalPreparation>>,
    // Proof/replay memo for this checking node only: dropped with the
    // checker, never published or shared with another node.
    pub(in crate::compiler::frontend) trait_scratch: std::cell::RefCell<tr::TraitScratch>,
    // Set only for closure bodies sharing the parent's nominal inputs.
    pub(in crate::compiler::frontend) nominal_closure_body: bool,
    // Node-local ordinal table for closure-bearing nominal applications.
    pub(in crate::compiler::frontend) parent_local_nominals:
        std::cell::RefCell<Vec<nominal_inputs::parent_local::ParentLocalApplication>>,
    pub(super) scopes: Vec<HashMap<LocalKey, ir::LocalId>>,
    pub(super) lexical_scope: Option<super::super::semantic_scope::ScopeId>,
    pub(super) checked_closures: Vec<closures::check::CheckedClosure>,
    pub(super) closure_source_recipe: Option<sr::OriginalBodyId>,
    closure_signature_hints: HashMap<PathKey, ir::Signature>,
    pub(super) capture_bindings: std::collections::HashSet<LocalKey>,
    pub(super) capture_parent_scope: Option<super::super::semantic_scope::ScopeId>,
    pub(super) opaque_hidden: Vec<ir::OpaqueHiddenType>,
    pub(super) pending_references: Vec<PendingReferenceDecl>,
    pub(super) locals: Vec<ir::LocalDecl>,
    pub(super) exprs: Vec<ir::TypedExpr>,
    pub(super) call_evidence: Vec<ir::CallEvidence>,
    pub(super) declaration: Option<ir::DeclId>,
    pub(super) loops: Vec<LoopFrame>,
    pub(super) next_loop: usize,
    pub(super) reachable: bool,
    pub(super) result: ir::Ty,
    pub(super) inferred_closure_result: bool,
    pub(super) module: ModuleId,
    pub(super) count_queries: Vec<(ir::IntTy, ir::ExprId)>,
    pub(super) field_indices:
        std::cell::RefCell<HashMap<ir::NominalAppId, HashMap<NameKey, usize>>>,
    pub(super) applications: std::cell::RefCell<Vec<CheckedApplication>>,
    pub(super) inference_evidence_cache:
        std::cell::RefCell<HashMap<(usize, usize, usize), InferTerm>>,
    /// Nesting depth of enclosing `unsafe` blocks; raw pointer dereference
    /// requires at least one (E0133).
    pub(super) unsafe_depth: usize,
    /// Default binding mode of the pattern position being checked. `None` is
    /// move mode; implicit dereferences of matched references switch it to a
    /// borrow. Always restored after each implicit dereference.
    pub(super) binding_mode: std::cell::Cell<Option<ir::BorrowKind>>,
    /// Storage for borrowed value temporaries (`&P { .. }`, `&f()`), with
    /// whether the temporary is lifetime-extended by an enclosing `let`.
    /// Extended temporaries are declared by the enclosing block; others are
    /// declared by a block wrapping only their statement or initializer.
    pub(super) borrow_temporaries: Vec<(ir::LocalId, bool)>,
    /// Source addresses of borrow operands in extending `let` positions.
    pub(super) extending_borrows: Vec<usize>,
}
#[derive(Clone, Copy)]
pub(super) struct ApplicationExpectation<'a> {
    outer: Expectation<'a>,
    arguments: &'a [Type],
}
#[derive(Clone, Copy)]
pub(super) enum Expectation<'a> {
    None,
    HasType(&'a ir::Ty),
    /// Solved local evidence is not a source-written coercion site. In
    /// particular, it must not turn a move of `&mut T` into a reborrow.
    Inferred(&'a ir::Ty),
    /// A constructor's declared field shape, before replacing its type
    /// parameters. Substitution cannot invent a written reference context.
    SourceSchema(&'a ir::Ty, &'a InferTerm),
    Application(&'a ApplicationExpectation<'a>),
    CastableToInt(ir::IntTy),
}
impl<'a> Expectation<'a> {
    pub(super) fn hard(self) -> Option<&'a ir::Ty> {
        match self {
            Self::HasType(ty) | Self::Inferred(ty) | Self::SourceSchema(ty, _) => Some(ty),
            Self::Application(application) => application.outer.hard(),
            _ => None,
        }
    }
    fn coercion(self) -> Option<&'a ir::Ty> {
        match self {
            Self::HasType(ty) => Some(ty),
            Self::SourceSchema(actual, formal)
                if !matches!(actual, ir::Ty::Ref { .. })
                    || matches!(formal.as_ref(), InferKind::Ref(..)) =>
            {
                Some(actual)
            }
            Self::Application(application) => application.outer.coercion(),
            _ => None,
        }
    }
    fn inherited<'b>(self, ty: Option<&'b ir::Ty>) -> Expectation<'b>
    where
        'a: 'b,
    {
        match self {
            Self::Inferred(_) => ty.map_or(Expectation::None, Expectation::Inferred),
            Self::SourceSchema(_, formal) => ty.map_or(Expectation::None, |actual| {
                Expectation::SourceSchema(actual, formal)
            }),
            Self::Application(application) => application.outer.inherited(ty),
            _ => ty.into(),
        }
    }
    fn aggregate_child<'b>(self, ty: Option<&'b ir::Ty>, index: usize) -> Expectation<'b>
    where
        'a: 'b,
    {
        match self {
            Self::SourceSchema(_, formal) => {
                let child = match formal.as_ref() {
                    InferKind::Tuple(fields) => fields.get(index),
                    InferKind::Array(element, _) => Some(element),
                    _ => None,
                };
                match (ty, child) {
                    (Some(actual), Some(formal)) => Expectation::SourceSchema(actual, formal),
                    _ => ty.map_or(Expectation::None, Expectation::Inferred),
                }
            }
            _ => self.inherited(ty),
        }
    }
    pub(super) fn literal(
        self,
        literal: &IntegerLiteral,
        negative: bool,
    ) -> Result<ir::IntValue, String> {
        let cast_hint = match self {
            Self::CastableToInt(to) if literal.suffix.is_none() => Some(ir::Ty::Int(to)),
            _ => None,
        };
        resolve_integer(literal, negative, self.hard().or(cast_hint.as_ref())).map_err(|error| {
            if cast_hint.is_some() && error.starts_with("integer literal out of range") {
                // A cast is a soft hint, not a source type obligation. The
                // default source integer could legally be wider than its cast.
                format!("{error}; backward numeric inference is unsupported")
            } else {
                error
            }
        })
    }
}
impl<'a> From<Option<&'a ir::Ty>> for Expectation<'a> {
    fn from(ty: Option<&'a ir::Ty>) -> Self {
        ty.map_or(Self::None, Self::HasType)
    }
}

// Semantic type and possible normal completion are independent. In particular,
// an unreachable explicit block tail retains its type, unlike a pure never value.
#[derive(Clone)]
pub(super) struct Flow {
    pub(super) ty: ir::Ty,
    pub(super) completes: bool,
}
impl Flow {
    pub(super) fn value(ty: ir::Ty) -> Self {
        Self {
            ty,
            completes: true,
        }
    }
    pub(super) fn never() -> Self {
        Self {
            ty: ir::Ty::Never,
            completes: false,
        }
    }
}
pub(super) fn source_region_mode(
    regions: &RegionDecls,
    params: &[(BindingName, Type)],
    result: &Type,
    free: bool,
    budget: &ir::PlanBudget,
) -> Result<tr::RegionContractMode, String> {
    budget.charge_work(params.len() + 1)?;
    let legacy = free
        && regions.params.is_empty()
        && regions.outlives.is_empty()
        && regions.receiver.is_none()
        && !matches!(result, Type::Ref { .. })
        && params.iter().all(|(_, ty)| match ty {
            Type::Ref {
                region, referent, ..
            } => {
                region.is_none()
                    && !matches!(referent.as_ref(), Type::Ref { .. })
                    && !matches!(referent.as_ref(), Type::Slice(element) if matches!(element.as_ref(), Type::Ref { .. }))
            }
            _ => true,
        });
    Ok(if legacy {
        tr::RegionContractMode::LegacyUnnamedFreeNoReturn
    } else {
        tr::RegionContractMode::Positioned
    })
}
// Original names are retained by the source registry. Normalization can prune
// unused declarations, but a pruned named lifetime remains a rigid source name.
pub(super) fn original_source_regions(
    context: (&NameStore, &ir::PlanBudget),
    declaration: ir::DeclId,
    regions: &RegionDecls,
    params: &[(BindingName, Type)],
    result: &Type,
    contract: &ir::RefContract,
    receiver: ReceiverKind,
) -> Result<Arc<ir::SourceRegionEnv>, String> {
    let (names_owner, budget) = context;
    if regions.params.len() > ir::MAX_REGION_PARAMS {
        return Err("declared lifetime resource limit exceeded".into());
    }
    budget.charge_work(regions.params.len() + regions.outlives.len() + params.len() + 1)?;
    budget.retain_bytes(
        regions
            .params
            .len()
            .checked_mul(256)
            .and_then(|n| {
                regions
                    .outlives
                    .len()
                    .checked_mul(64)
                    .and_then(|m| n.checked_add(m))
            })
            .and_then(|n| n.checked_add(512))
            .ok_or("source lifetime allocation overflow")?,
    )?;
    let mut names = HashMap::new();
    for (index, name) in regions.params.iter().enumerate() {
        budget.charge_work(1)?;
        if names
            .insert(
                names_owner
                    .key(*name, LookupClass::LifetimeParameter)
                    .map_err(native_adapter::name_diagnostic)?,
                index,
            )
            .is_some()
        {
            return Err("duplicate source lifetime".into());
        }
    }
    let lookup = |name: &RegionName| -> Result<ir::SourceRegionId, String> {
        budget.charge_work(1)?;
        let RegionName::Named(name) = name else {
            return Err("unsupported: builtin source lifetime".into());
        };
        let key = names_owner
            .key(*name, LookupClass::LifetimeParameter)
            .map_err(native_adapter::name_diagnostic)?;
        Ok(ir::SourceRegionId {
            declaration,
            index: *names.get(&key).ok_or("undeclared source lifetime")? as u16,
        })
    };
    let mut normalized = vec![None; regions.params.len()];
    let mut bind = |name: Option<&RegionName>, region: ir::RegionParamId| -> Result<(), String> {
        if let Some(name) = name
            && !matches!(name, RegionName::Static)
        {
            let source = lookup(name)?;
            let slot = &mut normalized[source.index as usize];
            if slot.is_some_and(|old| old != region) {
                return Err("source lifetime normalization disagrees with canonical schema".into());
            }
            *slot = Some(region);
        }
        Ok(())
    };
    // A receiver is present even when its lifetime is elided. The source params
    // exclude it; canonical input positions retain its actual parameter index.
    let offset = usize::from(receiver != ReceiverKind::None);
    if matches!(receiver, ReceiverKind::Borrowed { .. }) {
        bind(
            regions.receiver.as_ref(),
            contract
                .input_regions
                .first()
                .ok_or("borrowed receiver missing canonical region")?
                .region,
        )?;
    }
    for (index, (_, source)) in params.iter().enumerate() {
        let mut ty = source;
        let mut depth = 0u8;
        while let Type::Ref {
            region, referent, ..
        } = ty
        {
            budget.charge_work(1)?;
            // Canonical row lookup is bounded and explicitly charged.
            let mut found = None;
            for row in contract.input_regions.iter() {
                budget.charge_work(1)?;
                if row.input.param == index + offset && row.input.position.depth == depth {
                    found = Some(row.region);
                    break;
                }
            }
            if let Some(canonical) = found {
                bind(region.as_ref(), canonical)?;
            } else if region.is_some() {
                return Err("named source input missing canonical region".into());
            }
            depth = depth
                .checked_add(1)
                .ok_or("source lifetime nesting overflow")?;
            ty = referent;
        }
    }
    let mut ty = result;
    let mut depth = 0usize;
    while let Type::Ref {
        region, referent, ..
    } = ty
    {
        budget.charge_work(1)?;
        let canonical = contract
            .results
            .get(depth)
            .ok_or("source result missing canonical region")?;
        bind(region.as_ref(), canonical.region)?;
        depth += 1;
        ty = referent;
    }
    let mut outlives = Vec::with_capacity(regions.outlives.len());
    for (longer, shorter) in &regions.outlives {
        outlives.push((lookup(longer)?, lookup(shorter)?));
    }
    Ok(Arc::new(ir::SourceRegionEnv {
        declaration,
        region_count: regions.params.len() as u16,
        outlives: outlives.into(),
        normalized: normalized.into(),
        carriers: Arc::from([]),
    }))
}
pub(super) fn canonical_ref_contract(
    context: (&NameStore, &ir::PlanBudget),
    regions: &RegionDecls,
    source_params: &[(BindingName, Type)],
    source_result: &Type,
    source: (ReceiverKind, tr::RegionContractMode),
    params: &[ir::Ty],
    result: &ir::Ty,
) -> Result<Arc<ir::RefContract>, String> {
    let (names_owner, budget) = context;
    let (receiver, mode) = source;
    if regions.params.len() > 64 || regions.outlives.len() > 4096 {
        return Err("declared lifetime resource limit exceeded".into());
    }
    budget.charge_work(regions.params.len() + regions.outlives.len() + source_params.len() + 1)?;
    let bytes = regions
        .params
        .len()
        .checked_mul(256)
        .and_then(|n| {
            source_params
                .len()
                .checked_mul(256)
                .and_then(|m| n.checked_add(m))
        })
        .and_then(|n| {
            regions
                .outlives
                .len()
                .checked_mul(32)
                .and_then(|m| n.checked_add(m))
        })
        .and_then(|n| n.checked_add(4096))
        .ok_or("region allocation overflow")?;
    let _scratch = budget.reserve_bytes(bytes)?;
    let mut ids = HashMap::new();
    for (i, name) in regions.params.iter().enumerate() {
        budget.charge_work(1)?;
        if ids
            .insert(
                names_owner
                    .key(*name, LookupClass::LifetimeParameter)
                    .map_err(native_adapter::name_diagnostic)?,
                ir::RegionParamId(i as u16),
            )
            .is_some()
        {
            return Err("duplicate lifetime parameter".to_owned());
        }
    }
    let lookup = |name: &RegionName| {
        budget.charge_work(1)?;
        let RegionName::Named(name) = name else {
            return Err("unsupported: builtin lifetime in reference contract".into());
        };
        let key = names_owner
            .key(*name, LookupClass::LifetimeParameter)
            .map_err(native_adapter::name_diagnostic)?;
        ids.get(&key)
            .copied()
            .ok_or_else(|| "undeclared lifetime".to_owned())
    };
    let mut edges = Vec::with_capacity(regions.outlives.len() + 128);
    for (long, short) in &regions.outlives {
        edges.push((lookup(long)?, lookup(short)?));
    }
    if mode == tr::RegionContractMode::LegacyUnnamedFreeNoReturn {
        return ir::normalize_ref_contract(&ir::RefContract::default(), budget);
    }
    let mut count = ids.len();
    let mut fresh = |name: Option<&RegionName>| -> Result<ir::RegionParamId, String> {
        if let Some(name) = name
            && !matches!(name, RegionName::Static)
        {
            return lookup(name);
        }
        if count >= ir::MAX_REGION_PARAMS {
            return Err("combined lifetime regions exceed supported limit64".into());
        }
        let id = ir::RegionParamId(count as u16);
        count += 1;
        Ok(id)
    };
    let injected = params
        .len()
        .checked_sub(source_params.len())
        .ok_or("source parameter arity mismatch")?;
    if injected > 1 || (injected == 0 && receiver != ReceiverKind::None) {
        return Err("invalid injected receiver arity".into());
    }
    let mut inputs = Vec::with_capacity(params.len().saturating_mul(2));
    let receiver_region = if matches!(receiver, ReceiverKind::Borrowed { .. }) {
        let region = fresh(regions.receiver.as_ref())?;
        inputs.push(ir::InputRegion {
            input: ir::InputReference {
                param: 0,
                position: ir::ReferencePosition { depth: 0 },
            },
            region,
        });
        Some(region)
    } else {
        None
    };
    // A reference Self type (`impl Tr for &T`) adds inner reference layers
    // to the receiver; each gets its own region outliving the outer one.
    if injected == 1 {
        let skip = usize::from(receiver_region.is_some());
        let layers = ir::root_reference_layers(&params[0])?;
        let mut previous = receiver_region;
        for depth in skip..layers {
            let region = fresh(None)?;
            if let Some(outer) = previous {
                edges.push((region, outer));
            }
            inputs.push(ir::InputRegion {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: depth as u8 },
                },
                region,
            });
            previous = Some(region);
        }
    }
    for (index, (_, source_ty)) in source_params.iter().enumerate() {
        let mut ty = source_ty;
        let mut previous = None;
        let mut depth = 0;
        while let Type::Ref {
            region, referent, ..
        } = ty
        {
            if depth >= 128 {
                return Err("reference depth exceeds supported limit128".into());
            }
            if matches!(region, Some(RegionName::Static)) {
                return Err("unsupported: 'static lifetimes on parameter references".into());
            }
            let region = fresh(region.as_ref())?;
            if let Some(outer) = previous {
                edges.push((region, outer));
            }
            inputs.push(ir::InputRegion {
                input: ir::InputReference {
                    param: index + injected,
                    position: ir::ReferencePosition { depth },
                },
                region,
            });
            previous = Some(region);
            depth += 1;
            ty = referent;
        }
    }
    // Slice-element occurrences participate in output elision, but repeated
    // uses of the same explicit named lifetime are one lifetime. Retain these
    // ids only for elision: their carrier authority lives in SourceRegionEnv.
    let mut slice_element_regions = Vec::new();
    for (_, ty) in source_params {
        let Type::Ref { referent, .. } = ty else {
            continue;
        };
        let Type::Slice(element) = referent.as_ref() else {
            continue;
        };
        let Type::Ref { region, .. } = element.as_ref() else {
            continue;
        };
        budget.charge_work(1)?;
        budget.retain_bytes(std::mem::size_of::<ir::RegionParamId>())?;
        slice_element_regions
            .try_reserve_exact(1)
            .map_err(|_| "slice lifetime elision allocation failed")?;
        slice_element_regions.push(fresh(region.as_ref())?);
    }
    let mut results = Vec::with_capacity(2);
    let mut ty = source_result;
    let mut previous = None;
    while let Type::Ref {
        region, referent, ..
    } = ty
    {
        if results.len() >= 128 {
            return Err("reference depth exceeds supported limit128".into());
        }
        let region = if let Some(RegionName::Static) = region {
            // `'static` outlives every input: an independent region that no
            // input reference can supply. Only origin-free data may flow out.
            fresh(None)?
        } else if let Some(name) = region {
            // Nominal input occurrences retain their own declaration-owned
            // carrier region rows. Do not equate their named lifetime with
            // the temporary reference used to access the nominal value.
            lookup(name)?
        } else if let Some(region) = receiver_region {
            region
        } else {
            if inputs.len() != 1
                || slice_element_regions
                    .iter()
                    .any(|region| *region != inputs[0].region)
            {
                return Err("missing lifetime specifier: elided reference output requires exactly one input reference lifetime position".into());
            }
            inputs[0].region
        };
        if let Some(outer) = previous {
            edges.push((region, outer));
        }
        results.push(ir::ResultReference {
            position: ir::ReferencePosition {
                depth: results.len() as u8,
            },
            region,
            returns: ir::ReturnContract::NoReference,
        });
        previous = Some(region);
        ty = referent;
    }
    budget.retain_bytes(
        inputs
            .len()
            .checked_mul(64)
            .and_then(|n| edges.len().checked_mul(32).and_then(|m| n.checked_add(m)))
            .and_then(|n| n.checked_add(512))
            .ok_or("region allocation overflow")?,
    )?;
    let raw = ir::RefContract {
        region_count: count as u16,
        input_regions: inputs.into(),
        results: results.into(),
        outlives: edges.into(),
    };
    let normalized = ir::normalize_ref_contract(&raw, budget)?;
    // These are raw declaration schemas, not executable signatures. Preserve
    // the source's region metadata while an associated type is unresolved;
    // the world must normalize the exact application before deriving return
    // and carrier authority. In particular, do not feed a symbolic projection
    // into the executable reference-contract walker or invent its topology.
    let mut projected = ir::contains_associated_types(result, budget)?;
    for param in params {
        projected |= ir::contains_associated_types(param, budget)?;
    }
    if projected {
        return Ok(normalized);
    }
    let results = ir::derive_return_contracts(params, result, &normalized, budget)?;
    budget.retain_bytes(std::mem::size_of::<ir::RefContract>() + 64)?;
    Ok(Arc::new(ir::RefContract {
        results,
        ..(*normalized).clone()
    }))
}

/// A slice carrier is admitted only as an exact shared or mutable root reference.
fn admit_slice_value_shape(ty: &ir::Ty) -> Result<(), String> {
    if ty.contains_slice() && ty.slice_ref_element().is_none() {
        return Err(if matches!(ty, ir::Ty::Slice { .. }) {
            BARE_SLICE_UNSUPPORTED.into()
        } else {
            "unsupported: slices nested in aggregate or reference types".into()
        });
    }
    Ok(())
}
pub(super) fn reject_slice_result(result: &ir::Ty) -> Result<(), String> {
    admit_slice_value_shape(result)
}
pub(super) const BARE_SLICE_UNSUPPORTED: &str = "unsupported: bare by-value slice type [T]; only &[T] and &mut [T] parameters and locals are admitted";

/// Source shape only. The lowered element requires authoritative IR admission.
pub(super) fn admit_shared_slice_kind(kind: ir::BorrowKind, slice: &Type) -> Result<(), String> {
    let Type::Slice(_) = slice else {
        return Err("internal: slice admission on non-slice type".into());
    };
    let _ = kind; // `&[T]` and `&mut [T]` (MUTABLE-SLICES.md)
    Ok(())
}

pub(super) fn source_reference_layers(ty: &Type) -> Result<usize, String> {
    let mut inner = ty;
    let mut layers = 0;
    while let Type::Ref { referent, .. } = inner {
        layers += 1;
        if layers > 128 {
            return Err("reference type nesting exceeds supported limit128".into());
        }
        inner = referent;
    }
    Ok(layers)
}
pub(super) fn checked_local_ty(ty: &Type, env: &TypeEnv) -> Result<ir::Ty, String> {
    let mut inner = ty;
    let mut named = false;
    for _ in 0..source_reference_layers(ty)? {
        if let Type::Ref {
            region, referent, ..
        } = inner
        {
            named |= region.is_some();
            inner = referent;
        }
    }
    if named {
        return Err("named lifetime annotations on locals are unsupported; use an elided local reference annotation".into());
    }
    checked_ty(ty, env)
}
pub(super) fn checked_ty(ty: &Type, env: &TypeEnv) -> Result<ir::Ty, String> {
    checked_ty_at(ty, env, true)
}
pub(super) fn checked_ty_at(ty: &Type, env: &TypeEnv, allow_ref: bool) -> Result<ir::Ty, String> {
    if let Some(plan) = &env.plan {
        if !allow_ref && matches!(ty, Type::Ref { .. }) {
            return Err("nested or aggregate references are unsupported".into());
        }
        let caller = env.facts.as_deref().map_or_else(
            || {
                env.param_env
                    .as_deref()
                    .map_or(ir::FactsCtx::Concrete, ir::FactsCtx::Universal)
            },
            ir::FactsCtx::Validated,
        );
        return plan.source_ty(ty, &env.params, caller, &plan.budget);
    }
    match ty {
        Type::SelfType => env
            .self_ty
            .clone()
            .ok_or_else(|| "Self outside impl".into()),
        Type::Projection(_) => {
            Err("unsupported: associated type projections require authoritative planning and normalization".into())
        }
        Type::Apply(_) | Type::Param(_) => {
            Err("generic types require authoritative declaration environment".into())
        }
        Type::Slice(_) => Err(BARE_SLICE_UNSUPPORTED.into()),
        Type::RawPtr { kind, pointee } => Ok(ir::Ty::RawPtr {
            kind: *kind,
            pointee: std::sync::Arc::new(checked_ty_at(pointee, env, false)?),
        }),
        Type::Dyn(_) => Err("unsupported: trait objects require the planning frontend".into()),
        Type::Ref { referent, .. } if matches!(referent.as_ref(), Type::Slice(_)) => {
            Err("unsupported: shared slices require the planning frontend".into())
        }
        Type::Ref { kind, referent, .. } => {
            if !allow_ref || matches!(referent.as_ref(), Type::Never | Type::Infer) {
                return Err("nested or aggregate reference types are unsupported in B1".into());
            }
            Ok(ir::Ty::Ref {
                kind: *kind,
                referent: std::sync::Arc::new(checked_ty_at(
                    referent,
                    env,
                    source_reference_layers(ty)? > 1,
                )?),
            })
        }
        Type::Array(element, len) => {
            if *len > ir::MAX_ARRAY_LEN {
                return Err("array length exceeds supported limit".into());
            }
            Ok(ir::Ty::Array {
                element: std::sync::Arc::new(checked_ty_at(element, env, false)?),
                len: *len,
            })
        }
        Type::Int(ty) => Ok(ir::Ty::Int(*ty)),
        Type::Float(ty) => Ok(ir::Ty::Float(*ty)),
        Type::Bool => Ok(ir::Ty::Bool),
        Type::Char => Ok(ir::Ty::Char),
        Type::Str => Ok(ir::Ty::Str),
        Type::Unit => Ok(ir::Ty::Unit),
        Type::Never => Ok(ir::Ty::Never),
        Type::Tuple(fields) if !fields.is_empty() => fields
            .iter()
            .map(|ty| checked_ty_at(ty, env, false))
            .collect::<Result<Vec<_>, _>>()
            .map(|fields| ir::Ty::Tuple(fields.into())),
        Type::Tuple(_) => Err("empty tuple types must use ()".into()),
        Type::Named(name) => {
            let key = name_scope::path_key(&*env.plan()?.program.names, name, Namespace::Type)?;
            env.structs
                .get(&key)
                .cloned()
                .map(ir::Ty::Struct)
                .or_else(|| env.enums.get(&key).cloned().map(ir::Ty::Enum))
                .ok_or_else(|| "unknown nominal type".into())
        }
        Type::Opaque(_) => Err("unsupported: opaque return requires canonical declaration schema".into()),
        Type::Infer => Err("signature types cannot be inferred".into()),
    }
}
pub(super) fn inhabited(ty: &ir::Ty) -> bool {
    let mut work = vec![ty];
    let mut seen = std::collections::HashSet::new();
    while let Some(ty) = work.pop() {
        match ty {
            ir::Ty::Array { element, len } => {
                if *len > 0 {
                    work.push(element);
                }
            }
            ir::Ty::Never => return false,
            ir::Ty::Tuple(fields) => work.extend(fields.iter()),
            ir::Ty::Struct(ty) if seen.insert(ty.def().id) => {
                work.extend(ty.def().fields.iter().map(|field| &field.ty))
            }
            _ => {}
        }
    }
    true
}
pub(super) fn coerces(got: &ir::Ty, want: &ir::Ty) -> bool {
    if *got == ir::Ty::Never || got == want {
        return true;
    }
    match (got, want) {
        (
            ir::Ty::Array {
                element: a,
                len: na,
            },
            ir::Ty::Array {
                element: b,
                len: nb,
            },
        ) if na == nb => coerces(a, b),
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) if a.len() == b.len() => {
            a.iter().zip(b.iter()).all(|(a, b)| coerces(a, b))
        }
        _ => false,
    }
}
/// Reference layers peeled from a primitive-method receiver.
const MAX_INTRINSIC_AUTODEREF: usize = 8;

impl<'a> Checker<'a> {
    pub(super) fn names(&self) -> Result<&NameStore, String> {
        let names = &self.signatures.plan()?.program.names;
        names.budget().check().map_err(|error| {
            super::super::source::SourceError::from_token(
                super::super::token_session::TokenError::Budget(error),
            )
            .to_string()
        })?;
        Ok(names)
    }
    pub(super) fn path_key(&self, path: &AstPath, namespace: Namespace) -> Result<PathKey, String> {
        name_scope::path_key(self.names()?, path, namespace)
    }
    fn nominal_decl(&self, path: &AstPath) -> Result<ir::DeclId, String> {
        self.signatures
            .plan()?
            .registry
            .nominals
            .get(&self.path_key(path, Namespace::Type)?)
            .copied()
            .ok_or_else(|| "unknown nominal declaration".into())
    }
    pub(super) fn nominal_source(
        &self,
        declaration: ir::DeclId,
    ) -> Result<Arc<NominalSchema>, String> {
        let plan = self.signatures.plan()?;
        plan.budget.charge_work(1)?;
        plan.state
            .lock()
            .map_err(|_| "generic plan context poisoned")?
            .schemas
            .get(&declaration)
            .cloned()
            .ok_or_else(|| "missing nominal source schema".into())
    }
    fn checked_struct_shape(&self, ty: &ir::StructTy) -> Result<StructShape, String> {
        let schema = self.nominal_source(ty.def().source_decl)?;
        let NominalSchema::Struct(source) = schema.as_ref() else {
            return Err("contract: struct has a non-struct source schema".into());
        };
        match source.shape {
            StructShape::Unit => {
                if !source.type_params.is_empty() || !ty.def().type_args.is_empty() {
                    return Err("unsupported: generic unit structs".into());
                }
                if !source.fields.is_empty() || !ty.def().fields.is_empty() {
                    return Err("contract: unit struct source schema has fields".into());
                }
                Ok(StructShape::Unit)
            }
            StructShape::Record => {
                if source.fields.len() != ty.def().fields.len() {
                    return Err("contract: record struct field schema length mismatch".into());
                }
                Ok(StructShape::Record)
            }
            StructShape::Tuple => {
                if source.fields.is_empty() {
                    return Err("unsupported: zero-field tuple structs".into());
                }
                if source.fields.len() != ty.def().fields.len() {
                    return Err("contract: tuple struct field schema length mismatch".into());
                }
                for (index, field) in source.fields.iter().enumerate() {
                    self.infer_work(0)?;
                    if !matches!(field.name, FieldName::Tuple(actual) if actual == index) {
                        return Err("contract: noncontiguous tuple struct field schema".into());
                    }
                }
                Ok(StructShape::Tuple)
            }
        }
    }
    fn tuple_pattern_type(
        &self,
        pattern: &Pattern,
        expected: &ir::Ty,
    ) -> Result<Option<ir::StructTy>, String> {
        if let Pattern::VariantRest { name, .. } = pattern {
            let plan = self.signatures.plan()?;
            let key = self.path_key(name, Namespace::Type)?;
            let Some(declaration) = plan.registry.nominals.get(&key).copied() else {
                return Ok(None);
            };
            if !matches!(
                self.nominal_source(declaration)?.as_ref(),
                NominalSchema::Struct(_)
            ) {
                return Ok(None);
            }
            let ir::Ty::Struct(ty) = self.positional_pattern_type(pattern, expected)? else {
                return Err("contract: tuple struct rest resolved to non-struct".into());
            };
            return Ok(Some(ty));
        }
        let Pattern::Variant {
            name,
            shape,
            fields,
            rest,
        } = pattern
        else {
            return Ok(None);
        };
        let plan = self.signatures.plan()?;
        let key = self.path_key(name, Namespace::Type)?;
        let Some(declaration) = plan.registry.nominals.get(&key).copied() else {
            return Ok(None);
        };
        let schema = self.nominal_source(declaration)?;
        let NominalSchema::Struct(source) = schema.as_ref() else {
            return Ok(None);
        };
        if source.shape != StructShape::Tuple {
            return Err("wrong struct constructor pattern shape".into());
        }
        let actual = self.nominal(name, Some(expected))?;
        let ir::Ty::Struct(ty) = &actual else {
            return Err("contract: tuple struct resolved to non-struct".into());
        };
        self.checked_struct_shape(ty)?;
        self.accessible(&actual)?;
        if !coerces(expected, &actual) {
            return Err("tuple struct pattern nominal mismatch".into());
        }
        if *shape != ir::VariantShape::Tuple || *rest || fields.len() != ty.def().fields.len() {
            return Err("unsupported: tuple struct pattern shape, rest or arity".into());
        }
        for (index, (selector, _)) in fields.iter().enumerate() {
            self.infer_work(0)?;
            if !matches!(selector, FieldSelector::Tuple(actual) if *actual == index) {
                return Err("contract: tuple struct pattern selectors are not contiguous".into());
            }
            self.field_type(&actual, selector)?;
        }
        Ok(Some(ty.clone()))
    }
    fn empty_tuple_rest_unsupported() -> String {
        super::super::source::SourceError {
            kind: super::super::source::SourceErrorKind::Unsupported,
            primary: None,
            message: "positional rest requires a nonempty tuple".into(),
            _diagnostic_lease: None,
            location: None,
        }
        .to_string()
    }
    fn refutable_enum_let_unsupported() -> String {
        super::super::source::SourceError {
            kind: super::super::source::SourceErrorKind::Unsupported,
            primary: None,
            message: "refutable enum patterns in irrefutable positions are unsupported".into(),
            _diagnostic_lease: None,
            location: None,
        }
        .to_string()
    }
    fn nested_refutable_literal_unsupported() -> String {
        super::super::source::SourceError {
            kind: super::super::source::SourceErrorKind::Unsupported,
            primary: None,
            message: "nested refutable literal patterns are unsupported".into(),
            _diagnostic_lease: None,
            location: None,
        }
        .to_string()
    }
    fn positional_pattern_type(
        &self,
        pattern: &Pattern,
        expected: &ir::Ty,
    ) -> Result<ir::Ty, String> {
        let (actual, fields) = match pattern {
            Pattern::TupleRest(fields) => {
                let ir::Ty::Tuple(types) = expected else {
                    if *expected == ir::Ty::Unit {
                        return Err(Self::empty_tuple_rest_unsupported());
                    }
                    if *expected == ir::Ty::Never {
                        return Err(
                            "unsupported: positional rest cannot infer omitted tuple arity".into(),
                        );
                    }
                    return Err("positional rest requires a nonempty structural tuple".into());
                };
                if types.is_empty() {
                    return Err("unsupported: positional rest requires a nonempty tuple".into());
                }
                (expected.clone(), fields)
            }
            Pattern::VariantRest { name, fields } => {
                let plan = self.signatures.plan()?;
                let key = self.path_key(name, Namespace::Type)?;
                let Some(declaration) = plan.registry.nominals.get(&key).copied() else {
                    self.enum_variant(name)?;
                    return Err("unsupported: enum tuple positional rest patterns".into());
                };
                let schema = self.nominal_source(declaration)?;
                let NominalSchema::Struct(source) = schema.as_ref() else {
                    return Err("expected tuple struct positional rest".into());
                };
                if source.shape != StructShape::Tuple {
                    return Err("positional rest constructor is not a tuple struct".into());
                }
                let actual = self.nominal(name, Some(expected))?;
                let ir::Ty::Struct(ty) = &actual else {
                    return Err("contract: tuple rest constructor has non-struct type".into());
                };
                self.checked_struct_shape(ty)?;
                if !self.visible(&ty.def().visibility)? {
                    return Err("private tuple struct rest constructor".into());
                }
                for field in ty.def().fields.iter() {
                    self.infer_work(0)?;
                    if !self.visible(&field.visibility)? {
                        return Err("private tuple struct rest constructor field".into());
                    }
                }
                if !coerces(expected, &actual) {
                    return Err("tuple struct rest pattern nominal mismatch".into());
                }
                (actual, fields)
            }
            _ => return Err("contract: expected positional rest pattern".into()),
        };
        self.accessible(&actual)?;
        let arity = match &actual {
            ir::Ty::Tuple(types) => types.len(),
            ir::Ty::Struct(ty) => ty.def().fields.len(),
            _ => return Err("contract: positional rest domain is not aggregate".into()),
        };
        let projection = fields.project(arity)?;
        let count = fields
            .prefix
            .len()
            .checked_add(fields.suffix.len())
            .ok_or("positional rest count overflow")?;
        self.signatures.plan()?.budget.charge_work(count)?;
        for (index, _) in projection.iter() {
            self.field_type(&actual, &FieldSelector::Tuple(index))?;
        }
        Ok(actual)
    }
    fn array_pattern_unsupported(message: &str) -> String {
        super::super::source::SourceError {
            kind: super::super::source::SourceErrorKind::Unsupported,
            primary: None,
            message: message.into(),
            _diagnostic_lease: None,
            location: None,
        }
        .to_string()
    }
    fn array_pattern_type<'t>(
        &self,
        pattern: &Pattern,
        ty: &'t ir::Ty,
    ) -> Result<(&'t ir::Ty, usize), String> {
        let ir::Ty::Array { element, len } = ty else {
            return Err(
                if matches!(
                    ty,
                    ir::Ty::Never | ir::Ty::Ref { .. } | ir::Ty::Slice { .. }
                ) {
                    Self::array_pattern_unsupported(
                        "array patterns require a known fixed array domain",
                    )
                } else {
                    "array pattern requires a fixed array".into()
                },
            );
        };
        if *len == 0 {
            return Err(Self::array_pattern_unsupported(
                "zero-length array patterns are unsupported",
            ));
        }
        if *len > ir::MAX_ARRAY_LEN {
            return Err("array pattern length exceeds limit".into());
        }
        self.accessible(ty)?;
        let count = match pattern {
            Pattern::Array(fields) => {
                if fields.is_empty() {
                    return Err(Self::array_pattern_unsupported(
                        "empty array patterns are unsupported",
                    ));
                }
                if fields.len() != *len {
                    return Err("array pattern arity mismatch".into());
                }
                fields.len()
            }
            Pattern::ArrayRest(fields) => {
                fields.project(*len)?;
                fields
                    .prefix
                    .len()
                    .checked_add(fields.suffix.len())
                    .ok_or("array pattern count overflow")?
            }
            _ => return Err("contract: expected array pattern".into()),
        };
        self.signatures.plan()?.budget.charge_work(count)?;
        Ok((element, *len))
    }
    fn validate_aggregate_pattern(
        &self,
        pattern: &Pattern,
        ty: &ir::Ty,
        depth: usize,
        work: &mut usize,
        binders: &mut usize,
    ) -> Result<(), String> {
        *work = work
            .checked_add(1)
            .ok_or("aggregate pattern work overflow")?;
        if *work > ir::MAX_PATTERN_PROJECTION_WORK || depth > ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("aggregate pattern work or depth exceeds limit".into());
        }
        self.infer_work(0)?;
        self.accessible(ty)?;
        if Self::implicitly_dereferenced(pattern, ty) {
            return self.implicit_deref(ty, &mut Vec::new(), |this, referent, _| {
                this.validate_aggregate_pattern(pattern, referent, depth + 1, work, binders)
            });
        }
        let pattern = self.pattern_source(pattern, ty)?;
        match pattern {
            Pattern::At { pattern, .. } => {
                *binders = binders
                    .checked_add(1)
                    .ok_or("aggregate binder count overflow")?;
                self.validate_aggregate_pattern(pattern, ty, depth + 1, work, binders)
            }
            Pattern::Ref { kind, pattern } => {
                self.explicit_deref(*kind, ty, &mut Vec::new(), |this, referent, _| {
                    this.validate_aggregate_pattern(pattern, referent, depth + 1, work, binders)
                })
            }
            Pattern::Array(fields) => {
                let (element, _) = self.array_pattern_type(pattern, ty)?;
                for field in fields {
                    self.validate_aggregate_pattern(field, element, depth + 1, work, binders)?;
                }
                Ok(())
            }
            Pattern::ArrayRest(fields) => {
                let (element, len) = self.array_pattern_type(pattern, ty)?;
                for (_, field) in fields.project(len)?.iter() {
                    self.validate_aggregate_pattern(field, element, depth + 1, work, binders)?;
                }
                Ok(())
            }
            Pattern::Wildcard => Ok(()),
            Pattern::Binding { .. } => {
                self.infer_storage(depth + 1)?;
                *binders = binders
                    .checked_add(1)
                    .ok_or("aggregate binder count overflow")?;
                if *binders > ir::MAX_PATTERN_BINDINGS {
                    return Err("pattern binding count exceeds limit".into());
                }
                Ok(())
            }
            Pattern::TupleRest(fields) | Pattern::VariantRest { fields, .. } => {
                let actual = self.positional_pattern_type(pattern, ty)?;
                let arity = match &actual {
                    ir::Ty::Tuple(types) => types.len(),
                    ir::Ty::Struct(ty) => ty.def().fields.len(),
                    _ => return Err("contract: positional rest aggregate type".into()),
                };
                let projection = fields.project(arity)?;
                self.signatures.plan()?.budget.charge_work(
                    fields
                        .prefix
                        .len()
                        .checked_add(fields.suffix.len())
                        .ok_or("positional rest count overflow")?,
                )?;
                for (index, pattern) in projection.iter() {
                    let (_, field_ty) = self.field_type(&actual, &FieldSelector::Tuple(index))?;
                    self.validate_aggregate_pattern(pattern, &field_ty, depth + 1, work, binders)?;
                }
                Ok(())
            }
            Pattern::Tuple(patterns) => {
                if patterns.is_empty() && *ty == ir::Ty::Unit {
                    return Ok(());
                }
                let ir::Ty::Tuple(fields) = ty else {
                    return Err("aggregate tuple pattern type mismatch".into());
                };
                if patterns.len() != fields.len() {
                    return Err("aggregate tuple pattern arity mismatch".into());
                }
                for (pattern, ty) in patterns.iter().zip(fields.iter()) {
                    self.validate_aggregate_pattern(pattern, ty, depth + 1, work, binders)?;
                }
                Ok(())
            }
            Pattern::Struct { fields, rest, .. } => {
                if self.pattern_ty(pattern, ty)? != *ty {
                    return Err("aggregate nominal pattern mismatch".into());
                }
                let ir::Ty::Struct(structure) = ty else {
                    return Err("expected aggregate struct pattern".into());
                };
                if *rest && self.checked_struct_shape(structure)? != StructShape::Record {
                    return Err("unsupported: aggregate match rest requires a record struct".into());
                }
                if !rest && fields.len() != structure.def().fields.len() {
                    return Err("aggregate record pattern missing fields".into());
                }
                for (field, pattern) in fields {
                    let (_, field_ty) = self.field_type(ty, &FieldSelector::Named(*field))?;
                    self.validate_aggregate_pattern(pattern, &field_ty, depth + 1, work, binders)?;
                }
                Ok(())
            }
            Pattern::Variant { fields, .. } => {
                let Some(tuple) = self.tuple_pattern_type(pattern, ty)? else {
                    return Err("unsupported: enum children in aggregate match patterns".into());
                };
                for (index, (_, pattern)) in fields.iter().enumerate() {
                    self.validate_aggregate_pattern(
                        pattern,
                        &tuple.def().fields[index].ty,
                        depth + 1,
                        work,
                        binders,
                    )?;
                }
                Ok(())
            }
            Pattern::Path(_)
            | Pattern::Bool(_)
            | Pattern::Int { .. }
            | Pattern::Char(_)
            | Pattern::Range { .. } => Err("unsupported: refutable aggregate match leaf".into()),
            Pattern::Apply { .. } => Err("pattern application normalization invariant".into()),
            Pattern::Alternatives(_) => Err("unsupported: nested or-patterns".into()),
        }
    }
    fn pending_pattern_bindings(&self) -> Result<PendingPatternBindings, String> {
        Ok(PendingPatternBindings {
            rows: super::super::token_session::BudgetVec::new(
                self.names()?.budget(),
                super::super::expansion_budget::Storage::Temporary,
            )
            .map_err(pattern_token_error)?,
        })
    }
    fn temporary_pattern_vec<T>(
        &self,
        capacity: usize,
    ) -> Result<(Vec<T>, super::super::expansion_budget::ByteLease), String> {
        let budget = self.names()?.budget().clone();
        let bytes = capacity
            .checked_mul(std::mem::size_of::<T>())
            .ok_or("pattern temporary capacity overflow")?;
        let mut lease = budget
            .reserve(
                super::super::expansion_budget::Storage::Temporary,
                u64::try_from(bytes).map_err(|_| "pattern temporary capacity overflow")?,
            )
            .map_err(|error| pattern_token_error(error.into()))?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity).map_err(|_| {
            pattern_token_error(super::super::token_session::TokenError::Budget(
                budget.poison_allocation(),
            ))
        })?;
        let actual = values
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .ok_or("pattern actual capacity overflow")?;
        lease
            .resize(u64::try_from(actual).map_err(|_| "pattern actual capacity overflow")?)
            .map_err(|error| pattern_token_error(error.into()))?;
        Ok((values, lease))
    }
    fn retained_pattern_vec<T>(&self, capacity: usize) -> Result<Vec<T>, String> {
        if capacity == 0 {
            return Ok(Vec::new());
        }
        let budget = self.names()?.budget().clone();
        let bytes = capacity
            .checked_mul(std::mem::size_of::<T>())
            .ok_or("pattern retained capacity overflow")?;
        self.charge_node(bytes)?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity).map_err(|_| {
            pattern_token_error(super::super::token_session::TokenError::Budget(
                budget.poison_allocation(),
            ))
        })?;
        let extra = values
            .capacity()
            .checked_sub(capacity)
            .and_then(|count| count.checked_mul(std::mem::size_of::<T>()))
            .ok_or("pattern actual capacity overflow")?;
        if extra != 0 {
            self.charge_node(extra)?;
        }
        Ok(values)
    }
    fn metered_pattern_projection(
        &self,
        path: &[ir::Projection],
    ) -> Result<
        (
            Vec<ir::Projection>,
            super::super::expansion_budget::ByteLease,
        ),
        String,
    > {
        self.signatures.plan()?.budget.charge_work(path.len())?;
        let (mut values, lease) = self.temporary_pattern_vec(path.len())?;
        values.extend_from_slice(path);
        Ok((values, lease))
    }
    fn pattern_scratch(&self) -> Result<PatternScratch, String> {
        let (path, lease) = self.temporary_pattern_vec(
            ir::MAX_PATTERN_PROJECTION_DEPTH
                .checked_add(2)
                .ok_or("pattern depth overflow")?,
        )?;
        Ok(PatternScratch {
            path,
            _lease: lease,
        })
    }
    fn decision_work(&self, pending: &mut PendingDecision, amount: usize) -> Result<(), String> {
        pending.work = pending
            .work
            .checked_add(amount)
            .ok_or("decision pattern work overflow")?;
        if pending.work > ir::MAX_PATTERN_PROJECTION_WORK {
            return Err("decision pattern work exceeds limit".into());
        }
        self.signatures.plan()?.budget.charge_work(amount)
    }
    fn decision_test(
        &self,
        pending: &mut PendingDecision,
        path: &[ir::Projection],
        kind: ir::PatternTestKind,
    ) -> Result<(), String> {
        if pending.tests.len() >= ir::MAX_PATTERN_BINDINGS {
            return Err("pattern test count exceeds limit".into());
        }
        for index in 0..pending.tests.len() {
            let previous = &pending.tests.as_slice()[index].projection;
            let cost = previous
                .len()
                .max(path.len())
                .checked_add(1)
                .ok_or("decision path cost overflow")?;
            self.decision_work(pending, cost)?;
            let same = pending.tests.as_slice()[index].projection == path;
            if same {
                return Err("duplicate decision pattern test path".into());
            }
        }
        let (projection, lease) = self.metered_pattern_projection(path)?;
        pending
            .tests
            .push(PendingDecisionTest {
                projection,
                kind,
                _projection_lease: lease,
            })
            .map_err(pattern_token_error)
    }
    #[inline(never)]
    fn collect_decision(
        &self,
        pattern: &Pattern,
        ty: &ir::Ty,
        path: &mut Vec<ir::Projection>,
        pending: &mut PendingDecision,
        depth: usize,
    ) -> Result<(), String> {
        if depth > ir::MAX_PATTERN_PROJECTION_DEPTH || path.len() > ir::MAX_PATTERN_PROJECTION_DEPTH
        {
            return Err("decision pattern depth exceeds limit".into());
        }
        self.decision_work(
            pending,
            path.len().checked_add(1).ok_or("decision path overflow")?,
        )?;
        self.accessible(ty)?;
        if Self::implicitly_dereferenced(pattern, ty) {
            return self.implicit_deref(ty, path, |this, referent, path| {
                this.collect_decision(pattern, referent, path, pending, depth + 1)
            });
        }
        let pattern = self.pattern_source(pattern, ty)?;
        match pattern {
            Pattern::Ref { kind, pattern } => {
                self.explicit_deref(*kind, ty, path, |this, referent, path| {
                    this.collect_decision(pattern, referent, path, pending, depth + 1)
                })?;
            }
            Pattern::At {
                name,
                mutable,
                by_ref,
                pattern: inner,
            } => {
                self.at_binding_admitted(*by_ref, ty)?;
                let binding = Pattern::Binding {
                    name: *name,
                    mutable: *mutable,
                    by_ref: *by_ref,
                };
                self.pattern_locals(
                    &binding,
                    ty,
                    path,
                    &mut std::collections::HashSet::new(),
                    &mut pending.bindings,
                    &mut pending.work,
                )?;
                self.collect_decision(inner, ty, path, pending, depth + 1)?;
            }
            Pattern::Wildcard => {}
            Pattern::Binding { .. } => self.pattern_locals(
                pattern,
                ty,
                path,
                &mut std::collections::HashSet::new(),
                &mut pending.bindings,
                &mut pending.work,
            )?,
            Pattern::Bool(value) => {
                if *ty != ir::Ty::Bool {
                    return Err("bool decision pattern type mismatch".into());
                }
                pending.refutable_payload = true;
                self.decision_test(pending, path, ir::PatternTestKind::Bool(*value))?;
            }
            Pattern::Int { literal, negative } => {
                if !matches!(ty, ir::Ty::Int(_)) {
                    return Err("integer decision pattern type mismatch".into());
                }
                pending.refutable_payload = true;
                self.decision_test(
                    pending,
                    path,
                    ir::PatternTestKind::Int(resolve_integer(literal, *negative, Some(ty))?),
                )?;
            }
            Pattern::Char(value) => {
                if *ty != ir::Ty::Char {
                    return Err("character decision pattern type mismatch".into());
                }
                pending.refutable_payload = true;
                self.decision_test(
                    pending,
                    path,
                    ir::PatternTestKind::Char(ir::CharValue {
                        value: u32::from(*value),
                    }),
                )?;
            }
            Pattern::Range { lo, hi, inclusive } => {
                let kind = range_pattern_test(lo.as_ref(), hi.as_ref(), *inclusive, ty)?;
                pending.refutable_payload = true;
                self.decision_test(pending, path, kind)?;
            }
            Pattern::Tuple(fields) => {
                if fields.is_empty() && *ty == ir::Ty::Unit {
                    return Ok(());
                }
                let ir::Ty::Tuple(types) = ty else {
                    return Err("decision tuple pattern type mismatch".into());
                };
                if fields.len() != types.len() {
                    return Err("decision tuple pattern arity mismatch".into());
                }
                for (index, (pattern, ty)) in fields.iter().zip(types.iter()).enumerate() {
                    path.push(ir::Projection::Field(index));
                    self.collect_decision(pattern, ty, path, pending, depth + 1)?;
                    path.pop();
                }
            }
            Pattern::TupleRest(fields) => {
                let actual = self.positional_pattern_type(pattern, ty)?;
                let ir::Ty::Tuple(types) = &actual else {
                    return Err("decision positional tuple type mismatch".into());
                };
                let projection = fields.project(types.len())?;
                self.decision_work(
                    pending,
                    fields
                        .prefix
                        .len()
                        .checked_add(fields.suffix.len())
                        .ok_or("positional selection count overflow")?,
                )?;
                for (index, pattern) in projection.iter() {
                    path.push(ir::Projection::Field(index));
                    self.collect_decision(pattern, &types[index], path, pending, depth + 1)?;
                    path.pop();
                }
            }
            Pattern::Array(fields) => {
                let (element, _) = self.array_pattern_type(pattern, ty)?;
                for (index, pattern) in fields.iter().enumerate() {
                    path.push(ir::Projection::ConstantIndex(index));
                    self.collect_decision(pattern, element, path, pending, depth + 1)?;
                    path.pop();
                }
            }
            Pattern::ArrayRest(fields) => {
                let (element, len) = self.array_pattern_type(pattern, ty)?;
                self.decision_work(
                    pending,
                    fields
                        .prefix
                        .len()
                        .checked_add(fields.suffix.len())
                        .ok_or("array selection count overflow")?,
                )?;
                for (index, pattern) in fields.project(len)?.iter() {
                    path.push(ir::Projection::ConstantIndex(index));
                    self.collect_decision(pattern, element, path, pending, depth + 1)?;
                    path.pop();
                }
            }
            Pattern::Struct { name, fields, rest } => {
                let actual = self.nominal(name, Some(ty))?;
                if &actual != ty {
                    return Err("decision record nominal mismatch".into());
                }
                let ir::Ty::Struct(structure) = &actual else {
                    return Err("expected decision struct pattern".into());
                };
                let shape = self.checked_struct_shape(structure)?;
                if shape == StructShape::Tuple {
                    return Err("unsupported: braced tuple struct patterns".into());
                }
                if shape == StructShape::Unit && (!fields.is_empty() || *rest) {
                    return Err("unsupported: unit struct pattern fields or rest".into());
                }
                if !rest && fields.len() != structure.def().fields.len() {
                    return Err("decision record pattern missing fields".into());
                }
                let mut selected = super::super::token_session::BudgetVec::new(
                    self.names()?.budget(),
                    super::super::expansion_budget::Storage::Temporary,
                )
                .map_err(pattern_token_error)?;
                for (field, pattern) in fields {
                    let (index, field_ty) =
                        self.field_type(&actual, &FieldSelector::Named(*field))?;
                    for old in selected.as_slice() {
                        self.decision_work(pending, 1)?;
                        if *old == index {
                            return Err("duplicate decision record field".into());
                        }
                    }
                    selected.push(index).map_err(pattern_token_error)?;
                    path.push(ir::Projection::Field(index));
                    self.collect_decision(pattern, &field_ty, path, pending, depth + 1)?;
                    path.pop();
                }
            }
            Pattern::Path(_) | Pattern::Variant { .. } | Pattern::VariantRest { .. } => {
                if let Some(tuple) = self.tuple_pattern_type(pattern, ty)? {
                    let actual = ir::Ty::Struct(tuple.clone());
                    match pattern {
                        Pattern::Variant { fields, .. } => {
                            for (index, (_, pattern)) in fields.iter().enumerate() {
                                let (_, field_ty) =
                                    self.field_type(&actual, &FieldSelector::Tuple(index))?;
                                path.push(ir::Projection::Field(index));
                                self.collect_decision(
                                    pattern,
                                    &field_ty,
                                    path,
                                    pending,
                                    depth + 1,
                                )?;
                                path.pop();
                            }
                        }
                        Pattern::VariantRest { fields, .. } => {
                            self.decision_work(
                                pending,
                                fields
                                    .prefix
                                    .len()
                                    .checked_add(fields.suffix.len())
                                    .ok_or("tuple selection count overflow")?,
                            )?;
                            for (index, pattern) in fields.project(tuple.def().fields.len())?.iter()
                            {
                                let (_, field_ty) =
                                    self.field_type(&actual, &FieldSelector::Tuple(index))?;
                                path.push(ir::Projection::Field(index));
                                self.collect_decision(
                                    pattern,
                                    &field_ty,
                                    path,
                                    pending,
                                    depth + 1,
                                )?;
                                path.pop();
                            }
                        }
                        _ => return Err("tuple decision pattern normalization invariant".into()),
                    }
                } else {
                    let (enumeration, variant, fields) = self.variant_pattern(pattern, ty)?;
                    if !path.is_empty() && enumeration.def().variants.len() > 1 {
                        pending.refutable_payload = true;
                    }
                    self.decision_test(
                        pending,
                        path,
                        ir::PatternTestKind::EnumVariant {
                            ty: enumeration.clone(),
                            variant,
                        },
                    )?;
                    let owner = enumeration.def().id;
                    path.push(ir::Projection::Downcast(ir::VariantId {
                        owner,
                        index: variant,
                    }));
                    for &(index, pattern) in fields.as_slice() {
                        path.push(ir::Projection::Field(index));
                        self.collect_decision(
                            pattern,
                            &enumeration.def().variants[variant].fields[index].ty,
                            path,
                            pending,
                            depth + 1,
                        )?;
                        path.pop();
                    }
                    path.pop();
                }
            }
            Pattern::Apply { .. } => {
                return Err("decision application normalization invariant".into());
            }
            Pattern::Alternatives(_) => return Err("unsupported: nested or-patterns".into()),
        }
        Ok(())
    }
    #[inline(never)]
    fn collect_complex_pattern(
        &self,
        pattern: &Pattern,
        ty: &ir::Ty,
    ) -> Result<PendingPatternPlan, String> {
        let tests = super::super::token_session::BudgetVec::new(
            self.names()?.budget(),
            super::super::expansion_budget::Storage::Temporary,
        )
        .map_err(pattern_token_error)?;
        let mut pending = PendingDecision {
            tests,
            bindings: self.pending_pattern_bindings()?,
            work: 0,
            refutable_payload: false,
        };
        let mut scratch = self.pattern_scratch()?;
        self.collect_decision(pattern, ty, &mut scratch.path, &mut pending, 0)?;
        let root = pending
            .tests
            .as_slice()
            .first()
            .and_then(|test| match &test.kind {
                ir::PatternTestKind::EnumVariant { ty, variant } if test.projection.is_empty() => {
                    Some((ty.clone(), *variant))
                }
                _ => None,
            });
        let coverage = if pending.refutable_payload {
            PatternCoverage::Predicate {
                root_variant: root.as_ref().map(|(_, variant)| *variant),
            }
        } else if let Some((_, variant)) = &root {
            PatternCoverage::Variant(*variant)
        } else {
            PatternCoverage::CatchAll
        };
        let test = if pending.tests.as_slice().is_empty() {
            SourcePatternTest::Aggregate(ty.clone())
        } else if pending.tests.len() == 1 && !pending.refutable_payload && root.is_some() {
            let (ty, variant) = root.ok_or("missing root variant proof")?;
            SourcePatternTest::EnumVariant { ty, variant }
        } else {
            SourcePatternTest::Decision {
                ty: ty.clone(),
                tests: pending.tests,
                coverage,
            }
        };
        Ok(PendingPatternPlan {
            test,
            bindings: pending.bindings,
        })
    }
    #[inline(never)]
    fn collect_pattern(
        &self,
        pattern: &Pattern,
        domain: &ir::Ty,
    ) -> Result<PendingPatternPlan, String> {
        if matches!(pattern, Pattern::Alternatives(_)) {
            return Err("unsupported: or-patterns are only admitted at match arm roots".into());
        }
        if Self::implicitly_dereferenced(pattern, domain) {
            return self.collect_complex_pattern(pattern, domain);
        }
        let pattern = self.pattern_source(pattern, domain)?;
        let mut bindings = self.pending_pattern_bindings()?;
        let test = match pattern {
            Pattern::Wildcard => SourcePatternTest::Wildcard,
            Pattern::Binding {
                by_ref: Some(_), ..
            }
            | Pattern::At { .. } => return self.collect_complex_pattern(pattern, domain),
            Pattern::Binding { .. } => {
                self.pattern_locals(
                    pattern,
                    domain,
                    &mut Vec::new(),
                    &mut std::collections::HashSet::new(),
                    &mut bindings,
                    &mut 0usize,
                )?;
                SourcePatternTest::Binding
            }
            Pattern::Bool(value) => {
                if *domain != ir::Ty::Bool {
                    return Err(format!("bool match pattern does not match {domain:?}"));
                }
                SourcePatternTest::Bool(*value)
            }
            Pattern::Int { literal, negative } => {
                if !matches!(domain, ir::Ty::Int(_)) {
                    return Err(format!("integer match pattern does not match {domain:?}"));
                }
                SourcePatternTest::Int(resolve_integer(literal, *negative, Some(domain))?)
            }
            Pattern::Tuple(fields) if fields.is_empty() => {
                if *domain != ir::Ty::Unit {
                    return Err(format!("unit match pattern does not match {domain:?}"));
                }
                SourcePatternTest::Unit
            }
            Pattern::TupleRest(_) if *domain == ir::Ty::Unit => {
                return Err(Self::empty_tuple_rest_unsupported());
            }
            Pattern::Apply { .. } => {
                return Err("pattern application normalization invariant".into());
            }
            _ => return self.collect_complex_pattern(pattern, domain),
        };
        Ok(PendingPatternPlan { test, bindings })
    }
    #[inline(never)]
    fn publish_pattern(&mut self, plan: PendingPatternPlan) -> Result<ir::MatchPattern, String> {
        self.publish_pattern_with_bindings(plan, None)
    }
    fn publish_pattern_with_bindings(
        &mut self,
        plan: PendingPatternPlan,
        previous: Option<&HashMap<NameKey, ir::LocalId>>,
    ) -> Result<ir::MatchPattern, String> {
        if matches!(plan.test, SourcePatternTest::Binding) {
            let [binding] = plan.bindings.rows.as_slice() else {
                return Err("contract: whole binding plan cardinality".into());
            };
            let binding = &binding.local;
            return self
                .pattern_local(binding.name, binding.ty.clone(), binding.mutable, previous)
                .map(ir::MatchPattern::Binding);
        }
        let decision_tests = if let SourcePatternTest::Decision { tests, .. } = &plan.test {
            let mut output = self.retained_pattern_vec(tests.len())?;
            for test in tests.as_slice() {
                self.signatures
                    .plan()?
                    .budget
                    .charge_work(test.projection.len() + 1)?;
                let mut projection = self.retained_pattern_vec(test.projection.len())?;
                projection.extend_from_slice(&test.projection);
                output.push(ir::PatternTest {
                    projection,
                    kind: test.kind.clone(),
                });
            }
            Some(output)
        } else {
            None
        };
        let mut bindings = self.retained_pattern_vec(plan.bindings.rows.len())?;
        for pending in plan.bindings.rows.as_slice() {
            let binding = &pending.local;
            self.signatures
                .plan()?
                .budget
                .charge_work(binding.projection.len())?;
            let mut projection = self.retained_pattern_vec(binding.projection.len())?;
            projection.extend_from_slice(&binding.projection);
            let local =
                self.pattern_local(binding.name, binding.ty.clone(), binding.mutable, previous)?;
            bindings.push(ir::PatternBinding {
                local,
                projection,
                borrow: binding.borrow,
            });
        }
        Ok(match plan.test {
            SourcePatternTest::Wildcard => ir::MatchPattern::Wildcard,
            SourcePatternTest::Bool(value) => ir::MatchPattern::Bool(value),
            SourcePatternTest::Int(value) => ir::MatchPattern::Int(value),
            SourcePatternTest::Unit => ir::MatchPattern::Unit,
            // Selecting through a reference without binding anything reads
            // nothing: it is exactly an irrefutable wildcard.
            SourcePatternTest::Aggregate(ir::Ty::Ref { .. }) if bindings.is_empty() => {
                ir::MatchPattern::Wildcard
            }
            SourcePatternTest::Aggregate(ty) => ir::MatchPattern::Aggregate { ty, bindings },
            SourcePatternTest::EnumVariant { ty, variant } => ir::MatchPattern::EnumVariant {
                ty,
                variant,
                bindings,
            },
            SourcePatternTest::Decision { ty, .. } => ir::MatchPattern::Decision {
                ty,
                tests: decision_tests.ok_or("missing published decision tests")?,
                bindings,
            },
            SourcePatternTest::Binding => return Err("contract: whole binding publication".into()),
        })
    }
    fn or_rows_need_expansion(
        &self,
        rows: &[Pattern],
        domain: &ir::Ty,
        guarded: bool,
    ) -> Result<bool, String> {
        if guarded
            || *domain == ir::Ty::Never
            || ir::validate_alternative_domain(domain, Some(&self.signatures.plan()?.budget))
                .is_err()
        {
            return Ok(true);
        }
        for row in rows {
            if !self.alternative_leaf(row, 0)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn alternative_leaf(&self, pattern: &Pattern, depth: usize) -> Result<bool, String> {
        let budget = &self.signatures.plan()?.budget;
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        Ok(match pattern {
            Pattern::Bool(_) | Pattern::Int { .. } | Pattern::Char(_) | Pattern::Wildcard => true,
            Pattern::Tuple(fields) => {
                let mut admitted = true;
                for field in fields {
                    admitted &= self.alternative_leaf(field, depth + 1)?;
                }
                admitted
            }
            _ => false,
        })
    }
    fn publish_alternative_row(
        &mut self,
        plan: PendingPatternPlan,
    ) -> Result<Vec<ir::PatternTest>, String> {
        let pattern = self.publish_pattern(plan)?;
        let kind = match pattern {
            ir::MatchPattern::Decision {
                tests, bindings, ..
            } if bindings.is_empty() => return Ok(tests),
            ir::MatchPattern::Wildcard | ir::MatchPattern::Unit => return Ok(Vec::new()),
            ir::MatchPattern::Aggregate { bindings, .. } if bindings.is_empty() => {
                return Ok(Vec::new());
            }
            ir::MatchPattern::Bool(value) => ir::PatternTestKind::Bool(value),
            ir::MatchPattern::Int(value) => ir::PatternTestKind::Int(value),
            _ => {
                return Err(
                    "unsupported: or-pattern bindings and nominal rows are unsupported".into(),
                );
            }
        };
        let mut tests = self.retained_pattern_vec(1)?;
        tests.push(ir::PatternTest {
            projection: Vec::new(),
            kind,
        });
        Ok(tests)
    }
    fn source_fields(
        &self,
        declaration: ir::DeclId,
        variant: Option<usize>,
    ) -> Result<Vec<FieldName>, String> {
        let source = self.nominal_source(declaration)?;
        let fields = match (source.as_ref(), variant) {
            (NominalSchema::Struct(source), None) => &source.fields,
            (NominalSchema::Enum(source), Some(index)) => {
                &source
                    .variants
                    .get(index)
                    .ok_or("unknown enum variant")?
                    .fields
            }
            _ => return Err("nominal source kind mismatch".into()),
        };
        self.infer_storage(fields.len())?;
        let mut out = Vec::with_capacity(fields.len());
        for field in fields {
            self.infer_work(0)?;
            if let FieldName::Named(name) = field.name {
                self.names()?
                    .check(name)
                    .map_err(native_adapter::name_diagnostic)?;
            }
            out.push(field.name);
        }
        Ok(out)
    }
    /// Whether a named/positional field exists on a struct declaration.
    pub(super) fn has_member(
        &self,
        declaration: ir::DeclId,
        field: &FieldSelector,
    ) -> Result<bool, String> {
        Ok(match field {
            FieldSelector::Tuple(index) => *index < self.source_fields(declaration, None)?.len(),
            FieldSelector::Named(name) => match self.member_index(declaration, None, *name) {
                Ok(_) => true,
                Err(error) if error == "unknown nominal field" => false,
                Err(error) => return Err(error),
            },
            _ => true,
        })
    }
    fn member_index(
        &self,
        declaration: ir::DeclId,
        variant: Option<usize>,
        name: AstIdent,
    ) -> Result<usize, String> {
        let key = self
            .names()?
            .key(name, LookupClass::Member)
            .map_err(native_adapter::name_diagnostic)?;
        for (index, field) in self
            .source_fields(declaration, variant)?
            .into_iter()
            .enumerate()
        {
            self.infer_work(0)?;
            if let FieldName::Named(field) = field
                && self
                    .names()?
                    .key(field, LookupClass::Member)
                    .map_err(native_adapter::name_diagnostic)?
                    == key
            {
                return Ok(index);
            }
        }
        Err("unknown nominal field".into())
    }
    fn variant_index(&self, declaration: ir::DeclId, name: AstIdent) -> Result<usize, String> {
        let key = self
            .names()?
            .key(name, LookupClass::Member)
            .map_err(native_adapter::name_diagnostic)?;
        let source = self.nominal_source(declaration)?;
        let NominalSchema::Enum(source) = source.as_ref() else {
            return Err("expected enum source".into());
        };
        for (index, variant) in source.variants.iter().enumerate() {
            self.infer_work(0)?;
            if self
                .names()?
                .key(variant.name, LookupClass::Member)
                .map_err(native_adapter::name_diagnostic)?
                == key
            {
                return Ok(index);
            }
        }
        Err("unknown enum variant".into())
    }
    fn split_variant(&self, path: &AstPath) -> Result<(AstPath, AstIdent), String> {
        path.check(self.names()?)
            .map_err(native_adapter::name_diagnostic)?;
        let member = *path.segments().last().ok_or("missing enum variant")?;
        let owner = path
            .semantic_parent(self.names()?)
            .map_err(native_adapter::name_diagnostic)?;
        Ok((owner, member))
    }
    fn self_path(&self, path: &AstPath) -> Result<bool, String> {
        path.check(self.names()?)
            .map_err(native_adapter::name_diagnostic)?;
        Ok(path.anchor() == PathAnchor::SelfType && path.is_empty())
    }
    fn visible(&self, visibility: &ir::Visibility) -> Result<bool, String> {
        visibility
            .accessible_from(&self.signatures.plan()?.program.modules, self.module)
            .map_err(module_diagnostic)
    }
    pub(super) fn new(signatures: &'a Signatures, result: ir::Ty, module: ModuleId) -> Self {
        Self {
            runtime_sites: Vec::new(),
            signatures,
            nominal_inputs: None,
            nominal_closure_body: false,
            parent_local_nominals: std::cell::RefCell::new(Vec::new()),
            nominal_blocked: std::cell::RefCell::new(None),
            trait_scratch: std::cell::RefCell::new(tr::TraitScratch::default()),
            scopes: vec![HashMap::new()],
            lexical_scope: None,
            checked_closures: Vec::new(),
            closure_source_recipe: None,
            closure_signature_hints: HashMap::new(),
            capture_bindings: std::collections::HashSet::new(),
            capture_parent_scope: None,
            inferred_closure_result: false,
            opaque_hidden: Vec::new(),
            pending_references: Vec::new(),
            locals: Vec::new(),
            exprs: Vec::new(),
            call_evidence: Vec::new(),
            declaration: None,
            loops: Vec::new(),
            next_loop: 0,
            reachable: true,
            result,
            module,
            count_queries: Vec::new(),
            field_indices: std::cell::RefCell::new(HashMap::new()),
            applications: std::cell::RefCell::new(Vec::new()),
            inference_evidence_cache: std::cell::RefCell::new(HashMap::new()),
            unsafe_depth: 0,
            binding_mode: std::cell::Cell::new(None),
            borrow_temporaries: Vec::new(),
            extending_borrows: Vec::new(),
            canonical: None,
        }
    }
    pub(super) fn charge_node(&self, bytes: usize) -> Result<(), String> {
        if let Some(plan) = &self.signatures.plan {
            plan.budget.charge_nodes(1)?;
            plan.budget.charge_work(1)?;
            plan.state
                .lock()
                .map_err(|_| "generic plan context poisoned")?
                .retain(&plan.budget, bytes)?;
        }
        Ok(())
    }
    pub(super) fn facts(&self) -> ir::FactsCtx<'a> {
        if let Some(facts) = &self.signatures.facts {
            return ir::FactsCtx::Validated(facts);
        }
        self.signatures
            .param_env
            .as_deref()
            .map_or(ir::FactsCtx::Concrete, ir::FactsCtx::Universal)
    }
    pub(super) fn push_application(
        &self,
        path: &GenericPath,
        namespace: Namespace,
    ) -> Result<(), String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("generic application requires declaration registry")?;
        let app = plan.application(
            path,
            namespace,
            &self.signatures.params,
            self.facts(),
            &plan.budget,
        )?;
        let (_, source_args) = path.application()?;
        if namespace == Namespace::Value
            && let Some(declaration) = plan.registry.functions.get(&app.0)
        {
            plan.registry
                .check_explicit_impl_params(*declaration, source_args.len())?;
        }
        let regions = self.type_actual_regions(source_args, 0)?;
        self.infer_storage(1)?;
        self.applications.borrow_mut().push((app.0, app.1, regions));
        Ok(())
    }
    pub(super) fn source_region(&self, name: &RegionName) -> Result<ir::ActualRegion, String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("source region requires registry")?;
        let declaration = self
            .declaration
            .ok_or("source region requires original callable")?;
        let source = plan.callable_body(&plan.program, declaration)?;
        let RegionName::Named(name) = name else {
            return Err("unsupported: builtin lifetime actual".into());
        };
        let key = self
            .names()?
            .key(*name, LookupClass::LifetimeParameter)
            .map_err(native_adapter::name_diagnostic)?;
        for (index, declared) in source.regions.params.iter().enumerate() {
            plan.budget.charge_work(1)?;
            if self
                .names()?
                .key(*declared, LookupClass::LifetimeParameter)
                .map_err(native_adapter::name_diagnostic)?
                == key
            {
                return Ok(ir::ActualRegion::Named(ir::SourceRegionId {
                    declaration,
                    index: index as u16,
                }));
            }
        }
        Err("undeclared lifetime in explicit type argument".into())
    }
    pub(super) fn type_actual_regions(
        &self,
        args: &[Type],
        offset: usize,
    ) -> Result<Arc<[ir::TypeActualRegion]>, String> {
        let mut rows = Vec::new();
        for (argument, arg) in args.iter().enumerate() {
            self.infer_work(0)?;
            let mut ty = arg;
            let mut depth = 0u8;
            while let Type::Ref {
                region, referent, ..
            } = ty
            {
                self.infer_work(depth as usize)?;
                self.charge_node(4 * std::mem::size_of::<ir::TypeActualRegion>() + 64)?;
                rows.push(ir::TypeActualRegion {
                    argument: argument
                        .checked_add(offset)
                        .ok_or("type argument index overflow")?,
                    location: ir::CarrierLocation {
                        path: ir::CarrierPath(Arc::from([])),
                        channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth }),
                    },
                    source: region
                        .as_ref()
                        .map_or(Ok(ir::ActualRegion::Elided), |name| {
                            self.source_region(name)
                        })?,
                });
                depth = depth
                    .checked_add(1)
                    .ok_or("type argument reference nesting overflow")?;
                ty = referent;
            }
        }
        Ok(rows.into())
    }
    pub(super) fn checked_type_actual(&self, source: &Type) -> Result<ir::Ty, String> {
        let ty = self.source_type_actual(source)?;
        if ty.contains_slice() {
            return Err("unsupported: slice generic arguments".into());
        }
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("type actual requires registry")?;
        ir::validate_root_reference_actual(&ty, &plan.budget)?;
        // Validate source lifetime names now, before erasure and argument checking.
        self.type_actual_regions(std::slice::from_ref(source), 0)?;
        Ok(ty)
    }
    pub(super) fn nominal(&self, name: &AstPath, hint: Option<&ir::Ty>) -> Result<ir::Ty, String> {
        let key = self.path_key(name, Namespace::Type)?;
        if self.self_path(name)? {
            return self
                .signatures
                .self_ty
                .clone()
                .ok_or_else(|| "Self outside impl".into());
        }
        if let Some(ty) = hint {
            let matches = match ty {
                ir::Ty::Struct(t) => t.def().source_decl == self.nominal_decl(name)?,
                ir::Ty::Enum(t) => t.def().source_decl == self.nominal_decl(name)?,
                _ => false,
            };
            if matches {
                return Ok(ty.clone());
            }
        }
        if let Some(plan) = &self.signatures.plan {
            let decl = *plan
                .registry
                .nominals
                .get(&self.path_key(name, Namespace::Type)?)
                .ok_or("unknown nominal type")?;
            let args = if self
                .applications
                .borrow()
                .last()
                .is_some_and(|(owner, _, _)| owner == &key)
            {
                self.applications.borrow_mut().pop().unwrap().1
            } else {
                Vec::new()
            };
            return plan.intern_nominal(decl, &args, self.facts(), &plan.budget);
        }
        self.signatures
            .structs
            .get(&key)
            .cloned()
            .map(ir::Ty::Struct)
            .or_else(|| self.signatures.enums.get(&key).cloned().map(ir::Ty::Enum))
            .ok_or_else(|| "unknown nominal type".into())
    }
    pub(super) fn pattern_source<'p>(
        &self,
        pattern: &'p Pattern,
        expected: &ir::Ty,
    ) -> Result<&'p Pattern, String> {
        if let Pattern::Apply { path, pattern } = pattern {
            self.push_application(path, Namespace::Type)?;
            let result = (|| {
                let (name, _) = path.application()?;
                let _ = name;
                let plan = self.signatures.plan.as_ref().ok_or("missing plan")?;
                let (owner, args) = plan.application(
                    path,
                    Namespace::Type,
                    &self.signatures.params,
                    self.facts(),
                    &plan.budget,
                )?;
                self.names()?
                    .budget()
                    .charge(
                        super::super::expansion_budget::Counter::Work,
                        path.segments.len().saturating_add(1) as u64,
                    )
                    .map_err(|error| {
                        super::super::source::SourceError::from_token(
                            super::super::token_session::TokenError::Budget(error),
                        )
                        .to_string()
                    })?;
                let decl = *plan
                    .registry
                    .nominals
                    .get(&owner)
                    .ok_or("generic pattern is not nominal")?;
                let actual = plan.intern_nominal(decl, &args, self.facts(), &plan.budget)?;
                let actual = self.realize_constructor_source(&actual)?;
                if !coerces(expected, &actual) {
                    return Err("generic pattern application does not match scrutinee".into());
                }
                self.pattern_source(pattern, expected)
            })();
            self.applications.borrow_mut().pop();
            result
        } else {
            Ok(pattern)
        }
    }
    pub(super) fn enter_loop(&mut self, accepts_value: bool) -> ir::LoopId {
        let id = ir::LoopId(self.next_loop);
        self.next_loop += 1;
        self.loops.push(LoopFrame {
            id,
            label: None,
            is_block: false,
            accepts_value,
            checking_condition: !accepts_value,
            break_ty: None,
            expected: None,
            expected_is_inferred: false,
            expected_source_schema: None,
            can_exit: false,
        });
        id
    }
    pub(super) fn label_key(&self, label: Option<AstIdent>) -> Result<Option<NameKey>, String> {
        let names = &self.signatures.plan()?.program.names;
        label
            .map(|label| {
                names
                    .key(label, LookupClass::Label)
                    .map_err(native_adapter::name_diagnostic)
            })
            .transpose()
    }
    pub(super) fn loop_target(
        &self,
        operation: &str,
        label: Option<AstIdent>,
    ) -> Result<usize, String> {
        let key = self.label_key(label)?;
        let index = if let Some(label) = label {
            let text = self
                .signatures
                .plan()?
                .program
                .names
                .spelling(label)
                .map_err(native_adapter::name_diagnostic)?;
            self.loops
                .iter()
                .rposition(|f| f.label == key)
                .ok_or_else(|| format!("undeclared label '{text}"))?
        } else {
            self.loops
                .len()
                .checked_sub(1)
                .ok_or_else(|| format!("{operation} outside a loop"))?
        };
        let frame = &self.loops[index];
        if frame.is_block && (label.is_none() || operation == "continue") {
            return Err(format!(
                "{operation} cannot target a labeled block without a matching break label"
            ));
        }
        if self.loops[index..].iter().any(|f| f.checking_condition) {
            return Err(format!(
                "{operation} in a while condition is unsupported (E0590)"
            ));
        }
        Ok(index)
    }
    pub(super) fn install_lexical_scope(
        &mut self,
        scope: Option<super::super::semantic_scope::ScopeId>,
    ) -> Result<(), String> {
        if let Some(scope) = scope {
            let plan = self.signatures.plan()?;
            let arena = plan.program.scopes.as_ref().ok_or_else(|| {
                scope_diagnostic(super::super::semantic_scope::ScopeError::InvalidHandle)
            })?;
            arena
                .check_names(&*plan.program.names)
                .map_err(scope_diagnostic)?;
            arena.check_function(scope).map_err(scope_diagnostic)?;
        }
        self.lexical_scope = scope;
        Ok(())
    }
    fn local_key(
        &self,
        input: &impl LocalInput,
        declaration: bool,
    ) -> Result<Option<LocalKey>, String> {
        let names = self.names()?;
        let Some(function) = self.lexical_scope else {
            let key = input.checked_key(names)?;
            if key.is_none() && !declaration {
                return Err("wildcard is not a local reference".into());
            }
            return Ok(key);
        };
        if matches!(input.checked_key(names)?, Some(LocalKey::Receiver)) {
            let arena = self
                .signatures
                .plan()?
                .program
                .scopes
                .as_ref()
                .ok_or_else(|| {
                    scope_diagnostic(super::super::semantic_scope::ScopeError::InvalidHandle)
                })?;
            return receiver_authority::key(
                arena,
                names,
                function,
                input.occurrence(names)?,
                declaration,
            )
            .map(Some)
            .map_err(scope_diagnostic);
        }
        let Some(occurrence) = input.occurrence(names)? else {
            return match input.checked_key(names)? {
                None if declaration => Ok(None),
                None => Err("wildcard is not a local reference".into()),
                Some(LocalKey::Receiver) => {
                    Err("unsupported: lexical macro receiver binding".into())
                }
                _ => Err(scope_diagnostic(
                    super::super::semantic_scope::ScopeError::UnrecordedOccurrence,
                )),
            };
        };
        let arena = self
            .signatures
            .plan()?
            .program
            .scopes
            .as_ref()
            .ok_or_else(|| {
                scope_diagnostic(super::super::semantic_scope::ScopeError::InvalidHandle)
            })?;
        let binding = if declaration {
            Some(
                arena
                    .declaration_for(names, occurrence)
                    .map_err(scope_diagnostic)?,
            )
        } else {
            let referenced_function = arena
                .reference_function(names, occurrence)
                .map_err(scope_diagnostic)?;
            if referenced_function != function
                && Some(referenced_function) != self.capture_parent_scope
            {
                return Err(scope_diagnostic(
                    super::super::semantic_scope::ScopeError::FunctionMismatch,
                ));
            }
            arena
                .reference_for(names, occurrence)
                .map_err(scope_diagnostic)?
        };
        if let Some(binding) = binding {
            if arena.binding_function(binding).map_err(scope_diagnostic)? != function
                && (declaration || !self.capture_bindings.contains(&LocalKey::Anchored(binding)))
            {
                return Err(scope_diagnostic(
                    super::super::semantic_scope::ScopeError::FunctionMismatch,
                ));
            }
            Ok(Some(LocalKey::Anchored(binding)))
        } else {
            Ok(None)
        }
    }
    fn reserve_scoped_local(&mut self, scope: usize) -> Result<(), String> {
        if self.lexical_scope.is_none() {
            return Ok(());
        }
        self.charge_node(std::mem::size_of::<(LocalKey, ir::LocalId)>() * 4)?;
        let budget = self.names()?.budget().clone();
        self.scopes[scope].try_reserve(1).map_err(|_| {
            super::super::source::SourceError::from_token(
                super::super::token_session::TokenError::Budget(budget.poison_allocation()),
            )
            .to_string()
        })
    }
    pub(super) fn local(
        &mut self,
        name: impl LocalInput,
        ty: ir::Ty,
        mutable: bool,
    ) -> Result<ir::LocalId, String> {
        let names = &self.signatures.plan()?.program.names;
        let key = self.local_key(&name, true)?;
        let text = name.diagnostic(names)?;
        if self.lexical_scope.is_some() && key.is_some() {
            self.reserve_scoped_local(self.scopes.len() - 1)?;
        }
        let local = self.temporary(&text, ty, mutable)?;
        if let Some(key) = key {
            self.scopes.last_mut().unwrap().insert(key, local);
        }
        Ok(local)
    }
    fn admit_slice_value(&self, ty: &ir::Ty) -> Result<(), String> {
        admit_slice_value_shape(ty)?;
        if let Some((_, element)) = ty.slice_ref_element() {
            ir::validate_shared_slice_element(
                element,
                self.facts(),
                &self.signatures.plan()?.budget,
            )?;
        }
        Ok(())
    }
    pub(super) fn temporary(
        &mut self,
        text: &str,
        ty: ir::Ty,
        mutable: bool,
    ) -> Result<ir::LocalId, String> {
        self.admit_slice_value(&ty)?;
        let local = ir::LocalId(self.locals.len());
        self.charge_node(std::mem::size_of::<ir::LocalDecl>() + text.len())?;
        self.locals.push(ir::LocalDecl {
            name: text.to_owned(),
            ty,
            mutable,
        });
        Ok(local)
    }
    pub(super) fn bind(
        &mut self,
        name: impl LocalInput,
        ty: ir::Ty,
    ) -> Result<ir::LocalId, String> {
        self.prove_nominal_bounds(&ty)?;
        let names = &self.signatures.plan()?.program.names;
        if let Some(key) = self.local_key(&name, true)?
            && self.scopes.last().unwrap().contains_key(&key)
        {
            return Err(format!("duplicate parameter {}", name.diagnostic(names)?));
        }
        self.local(name, ty, false)
    }
    pub(super) fn charge_pending_lookup(&self) -> Result<(), String> {
        if let Some(plan) = &self.signatures.plan {
            let work = self
                .pending_references
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(self.scopes.len()))
                .ok_or("pending reference lookup work overflow")?;
            plan.budget.charge_work(work)?;
        }
        Ok(())
    }
    pub(super) fn pending_reference(&self, n: impl LocalInput) -> Result<Option<usize>, String> {
        let Some(key) = self.local_key(&n, false)? else {
            return Ok(None);
        };
        self.charge_pending_lookup()?;
        for scope in (0..self.scopes.len()).rev() {
            if self.scopes[scope].contains_key(&key) {
                return Ok(None);
            }
            for (index, pending) in self.pending_references.iter().enumerate().rev() {
                if pending.scope == scope
                    && pending.local.is_none()
                    && self.local_key(&pending.name, true)? == Some(key)
                {
                    return Ok(Some(index));
                }
            }
        }
        if matches!(key, LocalKey::Anchored(_)) {
            return Err(scope_diagnostic(
                super::super::semantic_scope::ScopeError::UnrecordedOccurrence,
            ));
        }
        Ok(None)
    }
    pub(super) fn lookup_optional(
        &self,
        n: impl LocalInput,
    ) -> Result<Option<ir::LocalId>, String> {
        let names = &self.signatures.plan()?.program.names;
        let Some(key) = self.local_key(&n, false)? else {
            return Ok(None);
        };
        self.charge_pending_lookup()?;
        for scope in (0..self.scopes.len()).rev() {
            if let Some(local) = self.scopes[scope].get(&key) {
                return Ok(Some(*local));
            }
            for pending in &self.pending_references {
                if pending.scope == scope
                    && pending.local.is_none()
                    && self.local_key(&pending.name, true)? == Some(key)
                {
                    return Err(format!(
                        "deferred local {} requires an independently typed reference assignment before use",
                        n.diagnostic(names)?
                    ));
                }
            }
        }
        if matches!(key, LocalKey::Anchored(_)) {
            return Err(scope_diagnostic(
                super::super::semantic_scope::ScopeError::UnrecordedOccurrence,
            ));
        }
        Ok(None)
    }
    pub(super) fn lookup(&self, n: impl LocalInput) -> Result<ir::LocalId, String> {
        self.lookup_optional(&n)?.ok_or_else(|| {
            match self.names().and_then(|names| n.diagnostic(names)) {
                Ok(text) => format!("unresolved local {text}"),
                Err(error) => error,
            }
        })
    }
    pub(super) fn flow(&self, id: ir::ExprId) -> Flow {
        let e = &self.exprs[id.0];
        Flow {
            ty: e.ty.clone(),
            completes: e.completion == ir::Completion::Returns,
        }
    }
    pub(super) fn require(&self, got: &Flow, want: &ir::Ty) -> Result<(), String> {
        if matches!((&got.ty,want),(ir::Ty::Ref{kind:ir::BorrowKind::Mut,referent:a},ir::Ty::Ref{kind:ir::BorrowKind::Shared,referent:b}) if a==b)
        {
            return Err("implicit mutable-to-shared reference coercion/reborrowing is unsupported; use an explicit &*r reborrow".into());
        }
        if !coerces(&got.ty, want) && !self.nominal_input_types_equal(&got.ty, want)? {
            Err(format!("expected {want:?}, got {:?}", got.ty))
        } else {
            Ok(())
        }
    }
    pub(super) fn join(&self, a: &ir::Ty, b: &ir::Ty, context: &str) -> Result<ir::Ty, String> {
        if *a == ir::Ty::Never {
            return Ok(b.clone());
        }
        if *b == ir::Ty::Never || a == b {
            return Ok(a.clone());
        }
        if let (
            ir::Ty::Array {
                element: a,
                len: na,
            },
            ir::Ty::Array {
                element: b,
                len: nb,
            },
        ) = (a, b)
            && na == nb
        {
            return Ok(ir::Ty::Array {
                element: std::sync::Arc::new(self.join(a, b, context)?),
                len: *na,
            });
        }
        if let (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) = (a, b)
            && a.len() == b.len()
        {
            return a
                .iter()
                .zip(b.iter())
                .map(|(a, b)| self.join(a, b, context))
                .collect::<Result<Vec<_>, _>>()
                .map(|fields| ir::Ty::Tuple(fields.into()));
        }
        Err(format!("{context} differ: {a:?} and {b:?}"))
    }
    pub(super) fn accessible(&self, ty: &ir::Ty) -> Result<(), String> {
        fn visit(checker: &Checker<'_>, ty: &ir::Ty, depth: usize) -> Result<(), String> {
            if depth > 128 {
                return Err("type visibility nesting exceeds limit128".into());
            }
            if let Some(plan) = &checker.signatures.plan {
                plan.budget.charge_work(1)?;
            }
            match ty {
                ir::Ty::Ref { referent, .. } => visit(checker, referent, depth + 1)?,
                ir::Ty::Array { element, .. } => visit(checker, element, depth + 1)?,
                ir::Ty::Tuple(fields) => {
                    for field in fields.iter() {
                        visit(checker, field, depth + 1)?;
                    }
                }
                ir::Ty::Enum(ty) if !checker.visible(&ty.def().visibility)? => {
                    return Err(format!(
                        "private type {} is inaccessible from the current module",
                        ty.def().name
                    ));
                }
                ir::Ty::Struct(ty) if !checker.visible(&ty.def().visibility)? => {
                    return Err(format!(
                        "private type {} is inaccessible from the current module",
                        ty.def().name
                    ));
                }
                _ => {}
            }
            Ok(())
        }
        visit(self, ty, 0)
    }
    pub(super) fn checked_place(&self, place: &Place) -> Result<(ir::Place, ir::Ty, bool), String> {
        if place.fields.len() > 128 {
            return Err("place projection nesting exceeds limit128".into());
        }
        let local = self.lookup(place.local)?;
        let declaration = &self.locals[local.0];
        let mut ty = declaration.ty.clone();
        let mut writable = declaration.mutable;
        let mut derefs = 0;
        let mut projection = Vec::with_capacity(place.fields.len());
        for selector in &place.fields {
            match selector {
                FieldSelector::Deref => {
                    derefs += 1;
                    if derefs > 128 {
                        return Err("reference dereference depth exceeds limit128".into());
                    }
                    if let ir::Ty::RawPtr { kind, pointee } = &ty {
                        if self.unsafe_depth == 0 {
                            return Err("E0133: dereference of raw pointer is unsafe and requires unsafe block".into());
                        }
                        writable = *kind == ir::BorrowKind::Mut;
                        ty = pointee.as_ref().clone();
                        projection.push(ir::Projection::Deref);
                        continue;
                    }
                    let ir::Ty::Ref { kind, referent } = &ty else {
                        return Err("dereference requires a reference".into());
                    };
                    writable = *kind == ir::BorrowKind::Mut;
                    ty = referent.as_ref().clone();
                    projection.push(ir::Projection::Deref);
                }
                FieldSelector::ConstantIndex(index) => {
                    ty = self.index_type(&ty, *index)?;
                    projection.push(ir::Projection::ConstantIndex(*index));
                }
                _ => {
                    if let ir::Ty::Ref { kind, referent } = &ty {
                        if derefs >= 128 {
                            return Err(
                                "multiple dereferences in one field place are unsupported".into()
                            );
                        }
                        if place.fields.len() >= 128 {
                            return Err(
                                "normalized place projection nesting exceeds limit128".into()
                            );
                        }
                        derefs += 1;
                        writable = *kind == ir::BorrowKind::Mut;
                        ty = referent.as_ref().clone();
                        projection.push(ir::Projection::Deref);
                    }
                    let (index, field) = self.field_type(&ty, selector)?;
                    ty = field;
                    projection.push(ir::Projection::Field(index));
                }
            }
        }
        self.accessible(&ty)?;
        Ok((ir::Place { local, projection }, ty, writable))
    }
    pub(super) fn validate_value_carriers(&self, ty: &ir::Ty) -> Result<(), String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("carrier validation requires registry")?;
        // Shape alone grants no lifetime, target, Copy or executable-body proof.
        ir::carrier_locations(ty, &plan.budget).map(|_| ())
    }
    pub(super) fn require_index(&self, flow: &Flow) -> Result<(), String> {
        self.require(flow,&ir::Ty::Int(ir::IntTy::Usize)).map_err(|_|"runtime index requires usize, without implicit i32 conversion; backward numeric inference is unsupported".into())
    }
    #[inline(never)]
    /// Self type of `<owner as Trait>::f(args)`; `Trait::f(args)` leaves the
    /// owner as `_`, inferred from the first argument (peeling one reference
    /// for borrowed receivers).
    pub(super) fn trait_call_owner(&self, owner: &Type, args: &[Expr]) -> Result<ir::Ty, String> {
        if !matches!(owner, Type::Infer) {
            return checked_local_ty(owner, self.signatures);
        }
        let first = args
            .first()
            .ok_or("unsupported: trait path call without arguments needs `<T as Trait>::`")?;
        let ty = match self.method_receiver_hint(first)? {
            // `&self`/`&mut self` receivers: Self is the referent.
            Some(ir::Ty::Ref { referent, .. }) => Some(referent.as_ref().clone()),
            Some(other) => Some(other),
            None => match first {
                Expr::Borrow { place, .. } => {
                    let source = place.clone();
                    self.checked_place(&source).ok().map(|(_, ty, _)| ty)
                }
                _ => None,
            },
        };
        ty.ok_or_else(|| {
            "unsupported: cannot infer Self type of trait path call; use `<T as Trait>::`".into()
        })
    }
    pub(super) fn method_receiver_hint(&self, receiver: &Expr) -> Result<Option<ir::Ty>, String> {
        if let Some(ty) = self.scalar_hint(receiver)? {
            return Ok(Some(ty));
        }
        let mut base = receiver;
        let mut depth = 0;
        while let Expr::Apply { value, .. } = base {
            self.infer_work(depth)?;
            depth += 1;
            base = value;
        }
        if matches!(
            base,
            Expr::Struct { .. }
                | Expr::TupleStruct { .. }
                | Expr::EnumVariant { .. }
                | Expr::Array(_)
                | Expr::Repeat { .. }
        ) {
            let evidence = self.inference_evidence(receiver, 0)?;
            // The receiver is typed independently, exactly like a standalone
            // constructor. Do not infer it from method inputs, results or impls.
            return Inference::empty(self)?.resolve(&evidence, true, 0);
        }
        if matches!(base, Expr::Borrow { .. } | Expr::BorrowValue { .. }) {
            let evidence = self.inference_evidence(receiver, 0)?;
            let ty = Inference::empty(self)?.resolve(&evidence, true, 0)?;
            return Ok(ty.filter(array_len_receiver::receiver_is_array));
        }
        Ok(None)
    }
    pub(super) fn scalar_hint(&self, e: &Expr) -> Result<Option<ir::Ty>, String> {
        Ok(match e {
            Expr::NativeSyscall(_) | Expr::TypeAlignment(_) => Some(ir::Ty::Int(ir::IntTy::Usize)),
            Expr::RawSlice { kind, arguments } => self.raw_slice_hint(*kind, arguments)?,
            Expr::UnexpandedMacro(_) => return Err(UNEXPANDED_MACRO_CHECK_ERROR.into()),
            Expr::Output(output) => {
                validate_output_site(output, self.names()?)?;
                Some(ir::Ty::Unit)
            }
            Expr::Panic(panic) => {
                validate_panic_site(panic, self.names()?)?;
                Some(ir::Ty::Never)
            }
            Expr::TraitCall {
                owner,
                trait_path,
                name,
                type_args,
                args,
            } => {
                let plan = self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("missing trait registry")?;
                let owner = self.trait_call_owner(owner, args)?;
                let trait_ref = plan.resolve_trait_use(
                    &owner,
                    trait_path,
                    self.signatures,
                    self.facts(),
                    &plan.budget,
                )?;
                let Some(actuals) = self.inferred_trait_method_actuals(
                    &trait_ref, name, type_args, args, None, false,
                )?
                else {
                    return Ok(None);
                };
                Some(
                    plan.select_trait(
                        &trait_ref,
                        name,
                        &actuals,
                        self.facts(),
                        false,
                        &plan.budget,
                    )?
                    .ok_or("E0277: trait obligation not proven")?
                    .1
                    .result,
                )
            }
            Expr::Apply { .. } => {
                let evidence = self.inference_evidence(e, 0)?;
                Inference::empty(self)?.resolve(&evidence, false, 0)?
            }
            Expr::AssociatedCall {
                owner,
                name,
                type_args,
                args,
            } => self.associated_result_hint(owner, name, type_args, args)?,
            Expr::MethodCall { receiver, .. }
                if leading_zeros_candidate(e, self.names()?)?
                    && matches!(self.scalar_hint(receiver)?, Some(ir::Ty::Int(_))) =>
            {
                Some(ir::Ty::Int(ir::IntTy::U32))
            }
            Expr::MethodCall { .. } if self.intrinsic_target(e)?.is_some() => self
                .intrinsic_target(e)?
                .and_then(ir::Intrinsic::signature)
                .map(|signature| signature.result),
            Expr::MethodCall { receiver, .. }
                if slice_len_candidate(e, self.names()?)?
                    && (matches!(receiver.as_ref(), Expr::Array(_) | Expr::Repeat { .. })
                        || self.method_receiver_hint(receiver)?.is_some_and(|ty| {
                            ty.descriptor_element().is_some()
                                || array_len_receiver::receiver_is_array(&ty)
                        }))
                    && !self.array_len_shadowed_by_method(e)? =>
            {
                Some(ir::Ty::Int(ir::IntTy::Usize))
            }
            Expr::MethodCall {
                receiver,
                name,
                type_args,
                args: call_args,
            } => {
                if let Some(owner) = self.method_receiver_hint(receiver)? {
                    self.method_selection_with_inference(&owner, name, type_args, call_args, None)?
                        .map(|selected| selected.signature.result)
                } else {
                    None
                }
            }
            Expr::Struct { .. } | Expr::TupleStruct { .. } | Expr::EnumVariant { .. } => {
                let evidence = self.inference_evidence(e, 0)?;
                Inference::empty(self)?.resolve(&evidence, false, 0)?
            }
            Expr::Cast { target, .. } => Some(checked_local_ty(target, self.signatures)?),
            Expr::Deref { value } => match self.scalar_hint(value)? {
                Some(ir::Ty::Ref { referent, .. }) => Some(referent.as_ref().clone()),
                Some(ir::Ty::RawPtr { pointee, .. }) => Some(pointee.as_ref().clone()),
                Some(owner) => self.deref_target_hint(&owner)?,
                None => match self.method_receiver_hint(value)? {
                    Some(owner) => self.deref_target_hint(&owner)?,
                    None => None,
                },
            },
            Expr::BorrowValue { kind, value } => {
                self.scalar_hint(value)?.map(|referent| ir::Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(referent),
                })
            }
            Expr::Borrow { kind, place } => {
                if let Some(ir::Ty::Ref { referent, .. }) = self.static_index_borrow_hint(place)? {
                    Some(ir::Ty::Ref {
                        kind: *kind,
                        referent,
                    })
                } else {
                    None
                }
            }
            Expr::Integer(literal) => literal.suffix.map(ir::Ty::Int),
            Expr::Float(literal) => literal.suffix.map(ir::Ty::Float),
            Expr::Bool(_) => Some(ir::Ty::Bool),
            Expr::Char(_) => Some(ir::Ty::Char),
            Expr::Str(_) => Some(ir::Ty::shared_str_ty()),
            Expr::ByteStr(bytes) => Some(ir::Ty::byte_string_ty(bytes.len())),
            Expr::ConstItem(_) | Expr::AssociatedConst(_) => Some(
                self.canonical
                    .ok_or("contract: constant hint requires sealed graph inputs")?
                    .constant_type(e)?,
            ),
            Expr::Tuple(fields) if fields.is_empty() => Some(ir::Ty::Unit),
            Expr::Var(name) => self.callable_value_hint(name)?,
            Expr::Call { name, args } => {
                let key = self.path_key(name, Namespace::Value)?;
                let plan = self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("call hint requires declaration registry")?;
                let declaration = *plan
                    .registry
                    .functions
                    .get(&key)
                    .ok_or("unresolved function in hint")?;
                if plan.param_env(declaration)?.arity() == 0 {
                    Some(
                        self.signatures
                            .functions
                            .get(&self.path_key(name, Namespace::Value)?)
                            .ok_or("missing function signature")?
                            .result
                            .clone(),
                    )
                } else if let Some(type_args) =
                    self.infer_call_args(declaration, args, None, false)?
                {
                    Some(
                        plan.instantiate_signature(
                            &ir::CallTarget {
                                declaration,
                                type_args: type_args.into(),
                            },
                            self.facts(),
                            &plan.budget,
                        )?
                        .result,
                    )
                } else {
                    None
                }
            }
            Expr::Unary { .. } | Expr::Binary { .. } => self.operator_result_hint(e)?,
            Expr::Field { base, field } => {
                if let Some(ty) = self.scalar_hint(base)? {
                    let receiver = if let ir::Ty::Ref { referent, .. } = &ty {
                        referent.as_ref()
                    } else {
                        &ty
                    };
                    if let ir::Ty::Struct(nominal) = receiver
                        && !self.has_member(nominal.def().source_decl, field)?
                        && let Some(target) = self.deref_target_hint(receiver)?
                    {
                        Some(self.field_type(&target, field)?.1)
                    } else {
                        Some(self.field_type(receiver, field)?.1)
                    }
                } else {
                    None
                }
            }
            // `s[a..b]` as a receiver behaves as the shared subslice handle.
            Expr::RangeIndex { base, .. } => match self.scalar_hint(base)? {
                Some(ir::Ty::Array { element, .. }) => Some(ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: std::sync::Arc::new(ir::Ty::Slice { element }),
                }),
                Some(ir::Ty::Ref { referent, .. }) => match referent.as_ref() {
                    ir::Ty::Array { element, .. } => Some(ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: std::sync::Arc::new(ir::Ty::Slice {
                            element: element.clone(),
                        }),
                    }),
                    ir::Ty::Slice { .. } => Some(ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: referent.clone(),
                    }),
                    _ => None,
                },
                _ => None,
            },
            Expr::Index { base, .. } | Expr::DynamicIndex { base, .. } => {
                match self.scalar_hint(base)? {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref().clone()),
                    Some(ty) => {
                        if let Some(element) = ty
                            .shared_slice_element()
                            .or_else(|| ty.shared_array_element_len().map(|(element, _)| element))
                        {
                            ir::validate_shared_slice_element(
                                element,
                                self.facts(),
                                &self.signatures.plan()?.budget,
                            )?;
                            Some(element.clone())
                        } else {
                            let literal;
                            let index = match e {
                                Expr::DynamicIndex { index, .. } => index.as_ref(),
                                Expr::Index { index, .. } => {
                                    literal = Expr::Integer(IntegerLiteral {
                                        magnitude: *index as u128,
                                        suffix: None,
                                    });
                                    &literal
                                }
                                _ => unreachable!(),
                            };
                            match self.index_reference_hint(&ty, index)? {
                                Some(ir::Ty::Ref { referent, .. }) => {
                                    Some(referent.as_ref().clone())
                                }
                                _ => None,
                            }
                        }
                    }
                    _ => None,
                }
            }
            // A literal or explicit cast carries its own type even behind
            // statements that diverge. Do not look up a general tail in the
            // outer scope: block-local bindings may shadow those names.
            Expr::Block(block)
                if block.statements.is_empty()
                    || matches!(
                        block.tail.as_deref(),
                        Some(
                            Expr::Integer(_)
                                | Expr::Float(_)
                                | Expr::Bool(_)
                                | Expr::Char(_)
                                | Expr::Cast { .. }
                        )
                    ) =>
            {
                match &block.tail {
                    Some(tail) => self.scalar_hint(tail)?,
                    None => Some(ir::Ty::Unit),
                }
            }
            _ => None,
        })
    }

    pub(super) fn deref_source(&self, mut id: ir::ExprId) -> bool {
        loop {
            match &self.exprs[id.0].kind {
                ir::ExprKind::Deref { .. } => return true,
                ir::ExprKind::Index { base, .. }
                | ir::ExprKind::Field { base, .. }
                | ir::ExprKind::DynamicIndex { base, .. } => id = *base,
                _ => return false,
            }
        }
    }
    /// A runtime-indexed place path rooted directly at a local through
    /// array indexing and fields only (`m[i][j]`, `s.rows[i][0]`), with no
    /// dereference. CFG addresses each runtime level in place, evaluating and
    /// bounds-checking indexes outermost first, and never copies a row.
    pub(super) fn nested_array_place(mut e: &Expr) -> bool {
        for _ in 0..ir::MAX_PATTERN_PROJECTION_DEPTH {
            match e {
                Expr::Var(_) => return true,
                Expr::DynamicIndex { base, .. }
                | Expr::Index { base, .. }
                | Expr::Field { base, .. } => e = base,
                _ => return false,
            }
        }
        false
    }
    pub(super) fn dynamic_place(e: &Expr) -> bool {
        match e {
            Expr::DynamicIndex { .. } => true,
            Expr::Index { base, .. } | Expr::Field { base, .. } => Self::dynamic_place(base),
            _ => false,
        }
    }
    pub(super) fn index_type(&self, ty: &ir::Ty, index: usize) -> Result<ir::Ty, String> {
        if matches!(ty, ir::Ty::Ref { .. }) {
            return Err("implicit reference autodereference is unsupported in B1; use an explicit (*r) place".into());
        }
        let ir::Ty::Array { element, len } = ty else {
            return Err("indexing requires an array".into());
        };
        if index >= *len {
            return Err(format!(
                "array index {index} out of bounds for length {len}"
            ));
        }
        self.accessible(element)?;
        Ok(element.as_ref().clone())
    }
    pub(super) fn field_type(
        &self,
        ty: &ir::Ty,
        field: &FieldSelector,
    ) -> Result<(usize, ir::Ty), String> {
        if matches!(ty, ir::Ty::Ref { .. }) {
            return Err("implicit reference autodereference is unsupported in B1; use an explicit (*r) place".into());
        }
        let (index, projected) = match (ty, field) {
            (ir::Ty::Tuple(fields), FieldSelector::Tuple(index)) => fields
                .get(*index)
                .cloned()
                .map(|ty| (*index, ty))
                .ok_or_else(|| {
                    format!(
                        "tuple field {index} is out of bounds for {} fields",
                        fields.len()
                    )
                }),
            (ir::Ty::Struct(ty), FieldSelector::Named(name)) => {
                if self.checked_struct_shape(ty)? != StructShape::Record {
                    return Err("named field projection requires a record struct".into());
                }
                let key = self
                    .names()?
                    .key(*name, LookupClass::Member)
                    .map_err(native_adapter::name_diagnostic)?;
                if !self.field_indices.borrow().contains_key(&ty.def().id) {
                    let fields = self.source_fields(ty.def().source_decl, None)?;
                    self.charge_node(
                        fields
                            .len()
                            .checked_mul(std::mem::size_of::<(NameKey, usize)>() * 2)
                            .ok_or("field index allocation overflow")?,
                    )?;
                    let mut indices = HashMap::new();
                    indices
                        .try_reserve(fields.len())
                        .map_err(|_| "field index allocation resource limit")?;
                    for (index, field) in fields.into_iter().enumerate() {
                        self.infer_work(0)?;
                        if let FieldName::Named(name) = field {
                            let key = self
                                .names()?
                                .key(name, LookupClass::Member)
                                .map_err(native_adapter::name_diagnostic)?;
                            if indices.insert(key, index).is_some() {
                                return Err("duplicate source field schema".into());
                            }
                        }
                    }
                    self.field_indices.borrow_mut().insert(ty.def().id, indices);
                }
                let index = *self
                    .field_indices
                    .borrow()
                    .get(&ty.def().id)
                    .and_then(|fields| fields.get(&key))
                    .ok_or("unknown nominal field")?;
                let field = &ty.def().fields[index];
                if !self.visible(&field.visibility)? {
                    return Err(format!("private field {name:?} in {}", ty.def().name));
                }
                Ok((index, field.ty.clone()))
            }
            (ir::Ty::Struct(ty), FieldSelector::Tuple(index)) => {
                if self.checked_struct_shape(ty)? != StructShape::Tuple {
                    return Err("numeric field projection requires a tuple struct".into());
                }
                let field = ty
                    .def()
                    .fields
                    .get(*index)
                    .ok_or("tuple struct field out of bounds")?;
                if !self.visible(&field.visibility)? {
                    return Err("private tuple struct field".into());
                }
                Ok((*index, field.ty.clone()))
            }
            _ => Err(format!("invalid field projection {field:?} on {ty:?}")),
        }?;
        self.accessible(&projected)?;
        Ok((index, projected))
    }
    /// A non-reference pattern matched against a reference implicitly
    /// dereferences it (RFC 2005 default binding modes, edition 2024 rules).
    /// `x @ p` binds the whole place by value only when it is Copy, so the
    /// subpattern's bindings never observe a moved-from value.
    fn at_binding_admitted(
        &self,
        by_ref: Option<ir::BorrowKind>,
        ty: &ir::Ty,
    ) -> Result<(), String> {
        if by_ref.is_none() && self.binding_mode.get().is_none() && !self.facts().must_copy(ty)? {
            return Err(
                "unsupported: by-value `@` bindings of non-Copy values (use `ref name @`)".into(),
            );
        }
        Ok(())
    }
    pub(super) fn implicitly_dereferenced(pattern: &Pattern, ty: &ir::Ty) -> bool {
        matches!(ty, ir::Ty::Ref { .. })
            && !matches!(
                pattern,
                Pattern::Wildcard | Pattern::Binding { .. } | Pattern::Ref { .. }
            )
    }
    /// Check `pattern` against the referent of `ty` with the default binding
    /// mode switched to a borrow, recording the dereference in `path`.
    pub(super) fn implicit_deref<R>(
        &self,
        ty: &ir::Ty,
        path: &mut Vec<ir::Projection>,
        f: impl FnOnce(&Self, &ir::Ty, &mut Vec<ir::Projection>) -> Result<R, String>,
    ) -> Result<R, String> {
        let ir::Ty::Ref { kind, referent } = ty else {
            return Err("contract: implicit dereference of a non-reference".into());
        };
        if path.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("pattern projection depth exceeds limit".into());
        }
        let previous = self.binding_mode.get();
        let mode = match (previous, kind) {
            (Some(ir::BorrowKind::Shared), _) | (_, ir::BorrowKind::Shared) => {
                ir::BorrowKind::Shared
            }
            _ => ir::BorrowKind::Mut,
        };
        self.binding_mode.set(Some(mode));
        path.push(ir::Projection::Deref);
        let result = f(self, referent, path);
        path.pop();
        self.binding_mode.set(previous);
        result
    }
    /// An explicit `&pat`/`&mut pat` dereference, only valid in move mode.
    pub(super) fn explicit_deref<R>(
        &self,
        kind: ir::BorrowKind,
        ty: &ir::Ty,
        path: &mut Vec<ir::Projection>,
        f: impl FnOnce(&Self, &ir::Ty, &mut Vec<ir::Projection>) -> Result<R, String>,
    ) -> Result<R, String> {
        if self.binding_mode.get().is_some() {
            return Err(
                "cannot explicitly dereference within an implicitly-borrowing pattern".into(),
            );
        }
        let ir::Ty::Ref {
            kind: actual,
            referent,
        } = ty
        else {
            return Err(format!(
                "mismatched types: reference pattern against {ty:?}"
            ));
        };
        if *actual != kind {
            return Err("mismatched types: reference pattern mutability differs".into());
        }
        if path.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("pattern projection depth exceeds limit".into());
        }
        path.push(ir::Projection::Deref);
        let result = f(self, referent, path);
        path.pop();
        result
    }
    /// `&value` where `value` is not a place: evaluate it into a fresh
    /// temporary declared by the enclosing block, then borrow that.
    pub(super) fn borrow_value(
        &mut self,
        kind: ir::BorrowKind,
        value: &Expr,
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        let source = value;
        if let Expr::RangeIndex {
            base,
            start,
            end,
            inclusive,
        } = value
        {
            return self.subslice_with_kind(
                kind,
                base,
                start.as_deref(),
                end.as_deref(),
                *inclusive,
            );
        }
        // The parser keeps literal index occurrences until the receiver type
        // is known. Preserve existing static array/raw-place borrowing when
        // that projection is legal, without routing it through runtime probes.
        if let Ok(place) = static_source_place(value)
            && self.checked_place(&place).is_ok()
        {
            return self.expr(&Expr::Borrow { kind, place });
        }
        if let Some(id) = self.borrow_indexed_place(kind, value)? {
            return Ok(id);
        }
        if place_expression(value) {
            return Err(
                "unsupported: borrowing runtime indexed or dereferenced places is unsupported"
                    .into(),
            );
        }
        if matches!(value, Expr::Field { .. } | Expr::Index { .. }) {
            // `&call().field` may name a place behind a returned reference
            // (autoderef), which a copied temporary would not alias.
            return Err(
                "unsupported: borrowing a field or element of a value expression is unsupported"
                    .into(),
            );
        }
        let hint = match (expected, value) {
            // `&[a, b]` at a `&[T]` site: the array literal gets element type T.
            (Some(ir::Ty::Ref { referent, .. }), Expr::Array(elements)) => {
                match referent.as_ref() {
                    ir::Ty::Slice { element } => Some(ir::Ty::Array {
                        element: element.clone(),
                        len: elements.len(),
                    }),
                    other => Some(other.clone()),
                }
            }
            (Some(ir::Ty::Ref { referent, .. }), Expr::Repeat { len, .. }) => {
                match referent.as_ref() {
                    ir::Ty::Slice { element } => Some(ir::Ty::Array {
                        element: element.clone(),
                        len: *len,
                    }),
                    other => Some(other.clone()),
                }
            }
            (Some(ir::Ty::Ref { referent, .. }), _)
                if !matches!(referent.as_ref(), ir::Ty::Slice { .. }) =>
            {
                Some(referent.as_ref().clone())
            }
            _ => None,
        };
        let value = self.expr_expected(value, hint.as_ref())?;
        let flow = self.flow(value);
        if !flow.completes {
            return Ok(value);
        }
        if !self.trivially_dropped(&flow.ty)? {
            return Err("unsupported: borrowing a temporary whose type has a destructor".into());
        }
        let local = self.temporary("$borrow_temporary", flow.ty.clone(), true)?;
        self.charge_node(std::mem::size_of::<ir::LocalId>())?;
        let extended = self
            .extending_borrows
            .contains(&(source as *const Expr as usize));
        self.borrow_temporaries.push((local, extended));
        let borrowed = self.borrow_expression(
            kind,
            ir::Place {
                local,
                projection: Vec::new(),
            },
        )?;
        let ty = self.exprs[borrowed.0].ty.clone();
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Block(ir::TypedBlock {
                statements: vec![ir::TypedStmt::Assign { local, value }],
                tail: Some(borrowed),
            }),
        });
        Ok(id)
    }
    /// Record borrow operands in extending positions of a `let` initializer:
    /// the initializer itself, operands of extending borrows, and fields of
    /// tuple, array, struct and variant constructors in extending positions.
    fn mark_extending_borrows(&mut self, expr: &Expr, depth: usize) -> Result<(), String> {
        if depth >= 128 {
            return Ok(());
        }
        self.charge_node(std::mem::size_of::<usize>())?;
        match expr {
            Expr::BorrowValue { value, .. } => {
                self.extending_borrows
                    .push(value.as_ref() as *const Expr as usize);
                self.mark_extending_borrows(value, depth + 1)?;
            }
            Expr::Apply { value, .. } => self.mark_extending_borrows(value, depth + 1)?,
            Expr::Tuple(fields) | Expr::Array(fields) | Expr::TupleStruct { fields, .. } => {
                for field in fields {
                    self.mark_extending_borrows(field, depth + 1)?;
                }
            }
            Expr::Struct { fields, .. } => {
                for (_, field) in fields {
                    self.mark_extending_borrows(field, depth + 1)?;
                }
            }
            Expr::EnumVariant { fields, .. } => {
                for (_, field) in fields {
                    self.mark_extending_borrows(field, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// Give the borrowed temporaries created by one statement their Rust
    /// scope. Extended temporaries are declared in the enclosing block. The
    /// others end with the statement: a `let` initializer is wrapped in its own
    /// block, and any other statement is wrapped together with its storage.
    fn scope_borrow_temporaries(
        &mut self,
        start: usize,
        statement: ir::TypedStmt,
        completes: bool,
        statements: &mut Vec<ir::TypedStmt>,
    ) -> Result<ir::TypedStmt, String> {
        if self.borrow_temporaries.len() == start {
            return Ok(statement);
        }
        let mut local_scope = Vec::new();
        for (local, extended) in self.borrow_temporaries.drain(start..) {
            if extended {
                statements.push(ir::TypedStmt::Declare { local });
            } else {
                local_scope.push(ir::TypedStmt::Declare { local });
            }
        }
        if local_scope.is_empty() {
            return Ok(statement);
        }
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let completion = if completes {
            ir::Completion::Returns
        } else {
            ir::Completion::Never
        };
        Ok(match statement {
            ir::TypedStmt::Let { local, value } => {
                let wrapped = self.temporary_block(local_scope, Some(value))?;
                ir::TypedStmt::Let {
                    local,
                    value: wrapped,
                }
            }
            ir::TypedStmt::LetPattern {
                value,
                ty,
                bindings,
            } => {
                let wrapped = self.temporary_block(local_scope, Some(value))?;
                ir::TypedStmt::LetPattern {
                    value: wrapped,
                    ty,
                    bindings,
                }
            }
            ir::TypedStmt::Assign { local, value } => {
                let wrapped = self.temporary_block(local_scope, Some(value))?;
                ir::TypedStmt::Assign {
                    local,
                    value: wrapped,
                }
            }
            ir::TypedStmt::LetElse { .. } => {
                return Err("unsupported: borrowed temporaries in let-else initializers".into());
            }
            other => {
                local_scope.push(other);
                let id = ir::ExprId(self.exprs.len());
                self.exprs.push(ir::TypedExpr {
                    ty: if completes {
                        ir::Ty::Unit
                    } else {
                        ir::Ty::Never
                    },
                    completion,
                    kind: ir::ExprKind::Block(ir::TypedBlock {
                        statements: local_scope,
                        tail: None,
                    }),
                });
                ir::TypedStmt::Expr(id)
            }
        })
    }
    fn temporary_block(
        &mut self,
        statements: Vec<ir::TypedStmt>,
        tail: Option<ir::ExprId>,
    ) -> Result<ir::ExprId, String> {
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let (ty, completion) = match tail {
            Some(tail) => (self.exprs[tail.0].ty.clone(), self.exprs[tail.0].completion),
            None => (ir::Ty::Unit, ir::Completion::Returns),
        };
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion,
            kind: ir::ExprKind::Block(ir::TypedBlock { statements, tail }),
        });
        Ok(id)
    }
    /// True when dropping a value of `ty` runs no user destructor.
    pub(super) fn trivially_dropped(&self, ty: &ir::Ty) -> Result<bool, String> {
        let Some(facts) = self.facts().validated() else {
            return Ok(true);
        };
        if !facts.has_normal_drop_impls() {
            return Ok(true);
        }
        Ok(matches!(
            facts
                .normal_drop_plan(ty)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?,
            super::super::traits::NormalDropPlan::Trivial
        ))
    }
    /// Recover only concrete pattern shape, without a general inference engine.
    pub(super) fn pattern_ty(&self, pattern: &Pattern, ty: &ir::Ty) -> Result<ir::Ty, String> {
        if !matches!(
            pattern,
            Pattern::Wildcard | Pattern::Binding { .. } | Pattern::Apply { .. }
        ) && let Some(facts) = self.facts().validated()
            && facts.has_normal_drop_impls()
            && matches!(
                facts
                    .normal_drop_plan(ty)
                    .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?,
                super::super::traits::NormalDropPlan::Destructor(_)
            )
        {
            return Err("unsupported: destructuring a normal Drop owner".into());
        }
        if Self::implicitly_dereferenced(pattern, ty) {
            self.implicit_deref(ty, &mut Vec::new(), |this, referent, _| {
                this.pattern_ty(pattern, referent)
            })?;
            return Ok(ty.clone());
        }
        match pattern {
            Pattern::Alternatives(_) => Err("unsupported: or-pattern outside match arm root".into()),
            Pattern::At { pattern: inner, .. } => self.pattern_ty(inner, ty),
            Pattern::Ref { kind, pattern: inner } => {
                self.explicit_deref(*kind, ty, &mut Vec::new(), |this, referent, _| {
                    this.pattern_ty(inner, referent)
                })?;
                Ok(ty.clone())
            }
            Pattern::Path(name) | Pattern::Variant { name,.. } | Pattern::VariantRest { name, .. } => {
                if let Some(tuple) = self.tuple_pattern_type(pattern, ty)? { return Ok(ir::Ty::Struct(tuple)); }
                let expected = if let ir::Ty::Enum(enumeration) = ty {
                    let (owner, _) = self.split_variant(name)?;
                    let actual = self.nominal(&owner, Some(ty))?;
                    if actual != ir::Ty::Enum(enumeration.clone()) { return Err("enum pattern owner mismatch".into()); }
                    actual
                } else { ir::Ty::Enum(self.enum_variant(name)?.0) };
                if !coerces(ty,&expected) { return Err(format!("enum pattern {name:?} does not match {ty:?}")); }
                Ok(expected)
            }
            Pattern::Apply {..} => self.pattern_ty(self.pattern_source(pattern,ty)?,ty),
            Pattern::Array(_) | Pattern::ArrayRest(_) => {
                self.array_pattern_type(pattern, ty)?;
                let mut work = 0;
                let mut binders = 0;
                self.validate_aggregate_pattern(pattern, ty, 0, &mut work, &mut binders)?;
                Ok(ty.clone())
            }
            Pattern::TupleRest(_) => self.positional_pattern_type(pattern, ty),
            Pattern::Bool(_) | Pattern::Int { .. } | Pattern::Char(_) | Pattern::Range { .. } => {
                Err("refutable literal pattern in let binding".into())
            }
            Pattern::Wildcard | Pattern::Binding { .. } => Ok(ty.clone()),
            Pattern::Tuple(patterns) => match ty {
                ir::Ty::Unit if patterns.is_empty() => Ok(ir::Ty::Unit),
                ir::Ty::Never if patterns.is_empty() => Ok(ir::Ty::Unit),
                ir::Ty::Never => patterns.iter().map(|pattern| {
                    if matches!(pattern, Pattern::Binding { .. } | Pattern::Wildcard) {
                        Err("unannotated never tuple pattern inference is unsupported; add a whole-pattern type annotation".into())
                    } else { self.pattern_ty(pattern, &ir::Ty::Never) }
                }).collect::<Result<Vec<_>, String>>().map(|fields| ir::Ty::Tuple(fields.into())),
                ir::Ty::Tuple(fields) if fields.len() == patterns.len() => patterns.iter().zip(fields.iter()).map(|(pattern, ty)| self.pattern_ty(pattern, ty)).collect::<Result<Vec<_>, _>>().map(|fields| ir::Ty::Tuple(fields.into())),
                _ => Err(format!("tuple pattern with {} fields does not match {ty:?}", patterns.len())),
            },
            Pattern::Struct { name, fields, rest } => {
                let expected = self.nominal(name,Some(ty))?;
                let ir::Ty::Struct(structure) = &expected else { return Err("expected struct pattern".into()); };
                let shape = self.checked_struct_shape(structure)?;
                if shape == StructShape::Tuple { return Err("unsupported: braced tuple struct patterns".into()); }
                if shape == StructShape::Unit && (!fields.is_empty() || *rest) {
                    return Err("unsupported: unit struct pattern fields or rest".into());
                }
                self.accessible(&expected)?;
                if !coerces(ty, &expected) { return Err(format!("struct pattern {name:?} does not match {ty:?}")); }
                Ok(expected)
            }
        }
    }
    pub(super) fn pattern_locals(
        &self,
        pattern: &Pattern,
        ty: &ir::Ty,
        path: &mut Vec<ir::Projection>,
        names: &mut std::collections::HashSet<NameKey>,
        pending: &mut impl PatternLocalSink,
        work: &mut usize,
    ) -> Result<(), String> {
        if path.len() > ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("pattern projection depth exceeds limit".into());
        }
        if Self::implicitly_dereferenced(pattern, ty) {
            return self.implicit_deref(ty, path, |this, referent, path| {
                this.pattern_locals(pattern, referent, path, names, pending, work)
            });
        }
        match pattern {
            Pattern::Alternatives(_) => {
                return Err("unsupported: or-pattern binding extraction".into());
            }
            Pattern::At {
                name,
                mutable,
                by_ref,
                pattern: inner,
            } => {
                self.at_binding_admitted(*by_ref, ty)?;
                let binding = Pattern::Binding {
                    name: *name,
                    mutable: *mutable,
                    by_ref: *by_ref,
                };
                self.pattern_locals(&binding, ty, path, names, pending, work)?;
                return self.pattern_locals(inner, ty, path, names, pending, work);
            }
            Pattern::Ref { kind, pattern } => {
                return self.explicit_deref(*kind, ty, path, |this, referent, path| {
                    this.pattern_locals(pattern, referent, path, names, pending, work)
                });
            }
            Pattern::Apply { .. } => {
                return self.pattern_locals(
                    self.pattern_source(pattern, ty)?,
                    ty,
                    path,
                    names,
                    pending,
                    work,
                );
            }
            Pattern::Wildcard => {}
            Pattern::Array(fields) => {
                let (element, _) = self.array_pattern_type(pattern, ty)?;
                for (index, field) in fields.iter().enumerate() {
                    path.push(ir::Projection::ConstantIndex(index));
                    self.pattern_locals(field, element, path, names, pending, work)?;
                    path.pop();
                }
            }
            Pattern::ArrayRest(fields) => {
                let (element, len) = self.array_pattern_type(pattern, ty)?;
                for (index, field) in fields.project(len)?.iter() {
                    path.push(ir::Projection::ConstantIndex(index));
                    self.pattern_locals(field, element, path, names, pending, work)?;
                    path.pop();
                }
            }
            Pattern::TupleRest(fields) => {
                let actual = self.positional_pattern_type(pattern, ty)?;
                let arity = match &actual {
                    ir::Ty::Tuple(types) => types.len(),
                    ir::Ty::Struct(ty) => ty.def().fields.len(),
                    _ => return Err("contract: positional rest binding type".into()),
                };
                let projection = fields.project(arity)?;
                self.signatures.plan()?.budget.charge_work(
                    fields
                        .prefix
                        .len()
                        .checked_add(fields.suffix.len())
                        .ok_or("positional rest count overflow")?,
                )?;
                for (index, pattern) in projection.iter() {
                    let (_, field_ty) = self.field_type(&actual, &FieldSelector::Tuple(index))?;
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(pattern, &field_ty, path, names, pending, work)?;
                    path.pop();
                }
            }
            Pattern::Path(_) | Pattern::Variant { .. } | Pattern::VariantRest { .. } => {
                if let Some(tuple) = self.tuple_pattern_type(pattern, ty)? {
                    if let Pattern::VariantRest { fields, .. } = pattern {
                        let projection = fields.project(tuple.def().fields.len())?;
                        self.signatures.plan()?.budget.charge_work(
                            fields
                                .prefix
                                .len()
                                .checked_add(fields.suffix.len())
                                .ok_or("positional rest count overflow")?,
                        )?;
                        for (index, pattern) in projection.iter() {
                            path.push(ir::Projection::Field(index));
                            self.pattern_locals(
                                pattern,
                                &tuple.def().fields[index].ty,
                                path,
                                names,
                                pending,
                                work,
                            )?;
                            path.pop();
                        }
                        return Ok(());
                    }
                    let Pattern::Variant { fields, .. } = pattern else {
                        return Err("contract: tuple pattern normalization".into());
                    };
                    for (index, (_, pattern)) in fields.iter().enumerate() {
                        path.push(ir::Projection::Field(index));
                        self.pattern_locals(
                            pattern,
                            &tuple.def().fields[index].ty,
                            path,
                            names,
                            pending,
                            work,
                        )?;
                        path.pop();
                    }
                    return Ok(());
                }
                let (enumeration, variant, fields) = self.variant_pattern(pattern, ty)?;
                if enumeration.def().variants.len() != 1 {
                    return Err(Self::refutable_enum_let_unsupported());
                }
                for &(index, pattern) in fields.as_slice() {
                    path.push(ir::Projection::Downcast(ir::VariantId {
                        owner: enumeration.def().id,
                        index: variant,
                    }));
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(
                        pattern,
                        &enumeration.def().variants[variant].fields[index].ty,
                        path,
                        names,
                        pending,
                        work,
                    )?;
                    path.pop();
                    path.pop();
                }
            }
            Pattern::Bool(_) | Pattern::Int { .. } | Pattern::Char(_) | Pattern::Range { .. } => {
                return Err(Self::nested_refutable_literal_unsupported());
            }
            Pattern::Binding {
                name,
                mutable,
                by_ref,
            } => {
                let mode = self.binding_mode.get();
                let borrow = match (by_ref, mode) {
                    (Some(_), Some(_)) => {
                        return Err(
                            "cannot explicitly borrow within an implicitly-borrowing pattern"
                                .into(),
                        );
                    }
                    (Some(kind), None) => Some(*kind),
                    (None, Some(_)) if *mutable => {
                        return Err(
                            "cannot mutably bind by value within an implicitly-borrowing pattern"
                                .into(),
                        );
                    }
                    (None, mode) => mode,
                };
                if borrow.is_none()
                    && path.contains(&ir::Projection::Deref)
                    && !self.facts().must_copy(ty)?
                {
                    return Err("cannot move a non-Copy value out of a reference".into());
                }
                if borrow.is_none()
                    && path
                        .iter()
                        .any(|step| matches!(step, ir::Projection::ConstantIndex(_)))
                    && !self.facts().must_copy(ty)?
                {
                    return Err(Self::array_pattern_unsupported(
                        "non-Copy bindings through array patterns are unsupported",
                    ));
                }
                let key = self
                    .names()?
                    .key(*name, LookupClass::Local)
                    .map_err(native_adapter::name_diagnostic)?;
                self.accessible(ty)?;
                if pending.len() >= ir::MAX_PATTERN_BINDINGS {
                    return Err("pattern binding count exceeds limit".into());
                }
                *work = work
                    .checked_add(1 + path.len())
                    .ok_or("pattern projection work overflow")?;
                if *work > ir::MAX_PATTERN_PROJECTION_WORK {
                    return Err("pattern projection work exceeds limit".into());
                }
                pending.push_local(
                    self,
                    PatternLocalInput {
                        key,
                        name: *name,
                        ty,
                        mutable: *mutable,
                        path,
                        borrow,
                    },
                    names,
                )?;
            }
            Pattern::Tuple(patterns) => {
                if patterns.is_empty() && *ty == ir::Ty::Unit {
                    return Ok(());
                }
                let ir::Ty::Tuple(fields) = ty else {
                    return Err(format!("tuple pattern does not match {ty:?}"));
                };
                if fields.len() != patterns.len() {
                    return Err("tuple pattern arity mismatch".into());
                }
                for (index, (pattern, ty)) in patterns.iter().zip(fields.iter()).enumerate() {
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(pattern, ty, path, names, pending, work)?;
                    path.pop();
                }
            }
            Pattern::Struct { name, fields, rest } => {
                let expected = self.pattern_ty(pattern, ty)?;
                if &expected != ty {
                    return Err(
                        "nested never struct pattern requires a concrete annotated shape".into(),
                    );
                }
                let ir::Ty::Struct(structure) = ty else {
                    return Err(format!("expected struct pattern {name:?}"));
                };
                let mut selected = std::collections::HashSet::new();
                for (field, pattern) in fields {
                    let (index, field_ty) = self.field_type(ty, &FieldSelector::Named(*field))?;
                    if !selected.insert(index) {
                        return Err(format!("duplicate struct pattern field {field:?}"));
                    }
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(pattern, &field_ty, path, names, pending, work)?;
                    path.pop();
                }
                if !rest && selected.len() != structure.def().fields.len() {
                    return Err(format!(
                        "missing fields in struct pattern {name:?}; use .. to omit fields"
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn expr_expected_when(
        &mut self,
        e: &Expr,
        reachable: bool,
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        self.expr_with_expectation_when(e, reachable, expected.into())
    }
    fn expr_with_expectation_when(
        &mut self,
        e: &Expr,
        reachable: bool,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.expr_with_expectation(e, expectation);
        self.reachable = previous;
        result
    }
    pub(super) fn constructor_type(
        &self,
        name: &AstPath,
        variant: Option<AstIdent>,
        fields: &[(InferField<'_>, &Expr)],
        expected: Option<&ir::Ty>,
    ) -> Result<ir::Ty, String> {
        let source_ty = self.constructor_type_unchecked(name, variant, fields, expected)?;
        let ty = self.realize_constructor_source(&source_ty)?;
        self.prove_nominal_bounds(&ty)?;
        Ok(ty)
    }
    fn constructor_type_unchecked(
        &self,
        name: &AstPath,
        variant: Option<AstIdent>,
        fields: &[(InferField<'_>, &Expr)],
        expected: Option<&ir::Ty>,
    ) -> Result<ir::Ty, String> {
        let key = self.path_key(name, Namespace::Type)?;
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("constructor requires planning context")?;
        if self.self_path(name)? {
            return self.nominal(name, None);
        }
        let decl = *plan
            .registry
            .nominals
            .get(&self.path_key(name, Namespace::Type)?)
            .ok_or("unknown nominal declaration")?;
        if plan.param_env(decl)?.arity() == 0
            || self
                .applications
                .borrow()
                .last()
                .is_some_and(|(owner, _, _)| owner == &key)
        {
            return self.nominal(name, None);
        }
        self.infer_constructor(name,variant,fields,expected,true)?.ok_or_else(||"unsupported generic inference: insufficient evidence; explicit type arguments required".into())
    }
    pub(super) fn enum_variant(&self, name: &AstPath) -> Result<(ir::EnumTy, usize), String> {
        let (owner, variant) = self.split_variant(name)?;
        let ir::Ty::Enum(ty) = self.nominal(&owner, None)? else {
            return Err("unknown enum".into());
        };
        self.accessible(&ir::Ty::Enum(ty.clone()))?;
        let index = self.variant_index(ty.def().source_decl, variant)?;
        Ok((ty, index))
    }
    fn select_enum_rest_fields<'p>(
        &self,
        fields: &'p PositionalRest,
        definition: &ir::VariantDef,
    ) -> Result<SelectedVariantFields<'p>, String> {
        if definition.shape != ir::VariantShape::Tuple {
            return Err("enum positional rest requires a tuple variant".into());
        }
        let projection = fields.project(definition.fields.len())?;
        let count = fields
            .prefix
            .len()
            .checked_add(fields.suffix.len())
            .ok_or("enum positional rest count overflow")?;
        if count > ir::MAX_PATTERN_PROJECTION_WORK {
            return Err("enum positional rest selection exceeds work limit".into());
        }
        self.signatures.plan()?.budget.charge_work(count)?;
        self.infer_storage(count)?;
        let budget = self.names()?.budget().clone();
        let mut selected = super::super::token_session::BudgetVec::new(
            &budget,
            super::super::expansion_budget::Storage::Temporary,
        )
        .map_err(|error| super::super::source::SourceError::from_token(error).to_string())?;
        for (index, pattern) in projection.iter() {
            self.accessible(&definition.fields[index].ty)?;
            selected.push((index, pattern)).map_err(|error| {
                super::super::source::SourceError::from_token(error).to_string()
            })?;
        }
        Ok(SelectedVariantFields::Positional(selected))
    }
    pub(super) fn variant_pattern<'p>(
        &self,
        pattern: &'p Pattern,
        expected: &ir::Ty,
    ) -> Result<ResolvedVariantPattern<'p>, String> {
        let (name, shape, fields, rest, positional) = match pattern {
            Pattern::Path(name) => (name, ir::VariantShape::Unit, &[][..], false, None),
            Pattern::Variant {
                name,
                shape,
                fields,
                rest,
            } => (name, *shape, fields.as_slice(), *rest, None),
            Pattern::VariantRest { name, fields } => {
                (name, ir::VariantShape::Tuple, &[][..], true, Some(fields))
            }
            _ => return Err("expected enum variant pattern".into()),
        };
        let (ty, variant) = if let ir::Ty::Enum(ty) = expected {
            let (owner, variant) = self.split_variant(name)?;
            if self.nominal(&owner, Some(expected))? != ir::Ty::Enum(ty.clone()) {
                return Err("variant pattern owner mismatch".into());
            }
            let index = self.variant_index(ty.def().source_decl, variant)?;
            (ty.clone(), index)
        } else {
            self.enum_variant(name)?
        };
        if *expected != ir::Ty::Enum(ty.clone()) {
            return Err(format!(
                "variant pattern {name:?} does not match {expected:?}"
            ));
        }
        let definition = &ty.def().variants[variant];
        if shape != definition.shape
            && !(definition.shape == ir::VariantShape::Unit
                && shape == ir::VariantShape::Struct
                && fields.is_empty())
        {
            return Err(format!(
                "variant pattern {name:?} has wrong constructor shape"
            ));
        }
        if let Some(fields) = positional {
            let selected = self.select_enum_rest_fields(fields, definition)?;
            return Ok((ty, variant, selected));
        }
        let mut seen = std::collections::HashSet::new();
        let mut selected = Vec::new();
        for (selector, pattern) in fields {
            let index = match selector {
                FieldSelector::Tuple(i) if definition.shape == ir::VariantShape::Tuple => *i,
                FieldSelector::Named(n) => {
                    self.member_index(ty.def().source_decl, Some(variant), *n)?
                }
                _ => return Err("invalid variant field selector".into()),
            };
            let field = definition
                .fields
                .get(index)
                .ok_or("variant pattern arity mismatch")?;
            if !seen.insert(index) {
                return Err("duplicate variant pattern field".into());
            }
            self.accessible(&field.ty)?;
            selected.push((index, pattern));
        }
        if !rest && seen.len() != definition.fields.len() {
            return Err("missing variant pattern fields".into());
        }
        Ok((ty, variant, SelectedVariantFields::Legacy(selected)))
    }
    pub(super) fn compound_value(
        &mut self,
        op: BinaryOp,
        ty: &ir::Ty,
        value: &Expr,
    ) -> Result<(ir::ExprId, Flow), String> {
        let shift = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
        let bitwise = matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor);
        if !matches!(
            op,
            BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::Shl
                | BinaryOp::Shr
        ) {
            return Err("invalid compound assignment operator".into());
        }
        if !matches!(ty, ir::Ty::Int(_))
            && !(matches!(ty, ir::Ty::Float(_)) && !shift && !bitwise)
            && !(bitwise && *ty == ir::Ty::Bool)
        {
            return Err(
                "compound assignment requires primitive integer or bitwise Bool destination".into(),
            );
        }
        let value = self.expr_expected(value, if shift { None } else { Some(ty) })?;
        let flow = self.flow(value);
        if shift {
            if flow.ty == ir::Ty::Never {
                return Err("compound shift on semantic Never is unsupported".into());
            }
            if !matches!(flow.ty, ir::Ty::Int(_)) {
                return Err("compound shift count must be an integer".into());
            }
            let ir::Ty::Int(ty) = ty else {
                return Err("compound shift destination must be integer".into());
            };
            if self.count_queries.len() >= COUNT_EVAL_WORK {
                return Err("compound shift query work limit exceeded".into());
            }
            self.count_queries.push((*ty, value));
        } else {
            if flow.ty == ir::Ty::Never {
                return Err("compound operator on semantic Never is unsupported".into());
            }
            self.require(&flow, ty)?;
        }
        Ok((value, flow))
    }
    /// Assignment through a reference returned by a genuine call. Stock
    /// order is preserved exactly: the RHS is evaluated and saved first, then
    /// the carrier call, then any runtime index, then the destination is
    /// read (compound) and written. Every subexpression runs once.
    pub(super) fn deref_assignment(
        &mut self,
        carrier: &Expr,
        fields: &[FieldSelector],
        index: Option<&Expr>,
        op: Option<BinaryOp>,
        value: &Expr,
        statements: &mut Vec<ir::TypedStmt>,
    ) -> Result<(ir::TypedStmt, bool), String> {
        let carrier_ty = self
            .scalar_hint(carrier)?
            .ok_or("unresolved reference call assignment carrier")?;
        let raw_deref = matches!(carrier_ty, ir::Ty::RawPtr { .. })
            && matches!(fields.first(), Some(FieldSelector::Deref));
        if !matches!(carrier_ty, ir::Ty::Ref { .. }) && !raw_deref {
            // Only a written `*` of a primitive value is certainly invalid
            // (E0614). By-value temporaries, raw pointers and user Deref
            // carriers are legal Rust outside this bounded slice.
            let primitive = matches!(
                carrier_ty,
                ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool | ir::Ty::Char | ir::Ty::Unit
            );
            if primitive && matches!(fields.first(), Some(FieldSelector::Deref)) {
                return Err("E0614: call assignment carrier must return a reference".into());
            }
            return Err(
                "unsupported: assignment through a call result that is not a reference".into(),
            );
        }
        // One paid projection, built once and later moved into the place.
        let (projection, destination, writable) =
            self.call_place_projection(&carrier_ty, fields)?;
        if projection.first() != Some(&ir::Projection::Deref) {
            return Err("unsupported: assignment into a by-value call temporary".into());
        }
        if !writable {
            return Err(
                "E0594: cannot assign to data behind a shared reference returned by a call".into(),
            );
        }
        let element = match index {
            None => None,
            Some(_) => {
                let ir::Ty::Array { element, .. } = &destination else {
                    return Err("runtime index assignment requires an array".into());
                };
                if op.is_none() {
                    // Existing plain runtime IndexAssign does not admit Deref.
                    return Err("unsupported: runtime indexed assignment through a call-returned reference; only compound operators are admitted".into());
                }
                Some(element.as_ref().clone())
            }
        };
        let target = element.as_ref().unwrap_or(&destination).clone();
        let (rhs, rhs_flow) = match op {
            Some(op) => self.compound_value(op, &target, value)?,
            None => {
                let rhs = self.expr_expected(value, Some(&target))?;
                let flow = self.flow(rhs);
                self.require(&flow, &target)?;
                (rhs, flow)
            }
        };
        let saved_ty = if rhs_flow.ty == ir::Ty::Never {
            target.clone()
        } else {
            rhs_flow.ty.clone()
        };
        let saved = self.temporary(
            &format!("$rhs{}", self.locals.len()),
            saved_ty.clone(),
            false,
        )?;
        self.charge_node(std::mem::size_of::<ir::TypedStmt>() * 3)?;
        self.plan_budget_guard()?;
        statements.push(ir::TypedStmt::Let {
            local: saved,
            value: rhs,
        });
        let pointer =
            self.expr_expected_when(carrier, self.reachable && rhs_flow.completes, None)?;
        let pf = self.flow(pointer);
        self.require(&pf, &carrier_ty)?;
        let handle = self.temporary(
            &format!("$carrier{}", self.locals.len()),
            carrier_ty.clone(),
            false,
        )?;
        self.plan_budget_guard()?;
        statements.push(ir::TypedStmt::Let {
            local: handle,
            value: pointer,
        });
        let place = ir::Place {
            local: handle,
            projection,
        };
        let loaded = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: saved_ty,
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Local(saved),
        });
        let completes = rhs_flow.completes && pf.completes;
        let result = match (index, op) {
            (None, None) => (
                ir::TypedStmt::AssignPlace {
                    place,
                    value: loaded,
                },
                completes,
            ),
            (None, Some(op)) => (
                ir::TypedStmt::AssignOp {
                    place,
                    op,
                    value: loaded,
                },
                completes,
            ),
            (Some(index), Some(op)) => {
                let index = self.expr_expected_when(
                    index,
                    self.reachable && completes,
                    Some(&ir::Ty::Int(ir::IntTy::Usize)),
                )?;
                let index_flow = self.flow(index);
                self.require_index(&index_flow)?;
                (
                    ir::TypedStmt::AssignIndexOp {
                        array: place,
                        index,
                        op,
                        value: loaded,
                    },
                    completes && index_flow.completes,
                )
            }
            (Some(_), None) => unreachable!("rejected above"),
        };
        self.plan_budget_guard()?;
        Ok(result)
    }
    /// `m[i1]..[ik] OP= value` with runtime rows (NESTED-INDEX.md). Stock order:
    /// value, then each runtime index outermost first with its bounds check
    /// before the next index is evaluated, then the final index and write.
    /// Lowered to existing IR: `Let $v`, per-level `Let $iN` plus an ignored
    /// runtime read (a bounds probe that lowers to IndexInspect), then
    /// AssignIndex/AssignIndexOp on a place with DynamicIndex($iN) steps.
    fn nested_index_assignment(
        &mut self,
        target: &Expr,
        op: Option<BinaryOp>,
        value: &Expr,
        statements: &mut Vec<ir::TypedStmt>,
    ) -> Result<(ir::TypedStmt, bool), String> {
        // Written path from the root local outward, including the final step.
        enum NestedStep<'e> {
            Static(&'e FieldSelector),
            ConstIndex(usize),
            Runtime(&'e Expr, &'e Expr),
        }
        let mut steps = Vec::new();
        let mut cursor: &Expr = target;
        let root = loop {
            if steps.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                return Err("place projection nesting exceeds limit128".into());
            }
            match cursor {
                Expr::Var(path) => break path,
                Expr::DynamicIndex { base, index, .. } => {
                    steps.push(NestedStep::Runtime(index, cursor));
                    cursor = base;
                }
                Expr::Index { base, index, .. } => {
                    steps.push(NestedStep::ConstIndex(*index));
                    cursor = base;
                }
                Expr::Field { base, field } => {
                    steps.push(NestedStep::Static(field));
                    cursor = base;
                }
                _ => {
                    return Err(
                        "unsupported: nested index assignment requires a local-rooted array".into(),
                    );
                }
            }
        };
        steps.reverse();
        let root = self.lookup(root)?;
        // Type the whole path before evaluating anything. Each runtime row must
        // be an array that is indexed again, or the final runtime index.
        let mut ty = self.locals[root.0].ty.clone();
        let last = steps.len() - 1;
        for (position, step) in steps.iter().enumerate() {
            ty = match step {
                NestedStep::Static(field) => {
                    if matches!(ty, ir::Ty::Ref { .. }) {
                        return Err(
                            "unsupported: nested index assignment through a reference".into()
                        );
                    }
                    self.field_type(&ty, field)?.1
                }
                NestedStep::ConstIndex(index) => self.index_type(&ty, *index)?,
                NestedStep::Runtime(..) => {
                    let element = match &ty {
                        ir::Ty::Array { element, .. } => element.as_ref().clone(),
                        ir::Ty::Ref { .. } => {
                            return Err(
                                "unsupported: nested index assignment through a reference".into()
                            );
                        }
                        _ => return Err("runtime index assignment requires an array".into()),
                    };
                    if position != last
                        && !(matches!(element, ir::Ty::Array { .. })
                            && matches!(
                                steps[position + 1],
                                NestedStep::Runtime(..) | NestedStep::ConstIndex(_)
                            ))
                    {
                        return Err("unsupported: field or whole-row access after a runtime index in an assignment target".into());
                    }
                    element
                }
            };
        }
        let destination = ty;
        if !matches!(
            destination,
            ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Char | ir::Ty::Bool | ir::Ty::Unit
        ) {
            return Err(
                "unsupported: nested runtime index assignment of a non-scalar element".into(),
            );
        }
        if !self.locals[root.0].mutable {
            return Err("runtime index assignment requires a mutable place".into());
        }
        // 1. value
        let (rhs, rhs_flow) = match op {
            Some(op) => self.compound_value(op, &destination, value)?,
            None => {
                let rhs = self.expr_expected(value, Some(&destination))?;
                let flow = self.flow(rhs);
                self.require(&flow, &destination)?;
                (rhs, flow)
            }
        };
        let saved_ty = if rhs_flow.ty == ir::Ty::Never {
            destination.clone()
        } else {
            rhs_flow.ty.clone()
        };
        let saved = self.temporary(
            &format!("$nv{}", self.locals.len()),
            saved_ty.clone(),
            false,
        )?;
        self.charge_node(std::mem::size_of::<ir::TypedStmt>())?;
        statements.push(ir::TypedStmt::Let {
            local: saved,
            value: rhs,
        });
        let mut completes = rhs_flow.completes;
        // 2. path steps in source order. Each interior runtime index is saved
        // and bounds-checked before the next index expression is evaluated.
        let usize_ty = ir::Ty::Int(ir::IntTy::Usize);
        let mut place = ir::Place {
            local: root,
            projection: Vec::new(),
        };
        let mut place_ty = self.locals[root.0].ty.clone();
        let mut final_index = None;
        for (position, step) in steps.iter().enumerate() {
            match step {
                NestedStep::Static(field) => {
                    let (index, field_ty) = self.field_type(&place_ty, field)?;
                    place.projection.push(ir::Projection::Field(index));
                    place_ty = field_ty;
                }
                NestedStep::ConstIndex(index) => {
                    place_ty = self.index_type(&place_ty, *index)?;
                    place.projection.push(ir::Projection::ConstantIndex(*index));
                }
                NestedStep::Runtime(index, original) => {
                    let index = self.expr_expected_when(
                        index,
                        self.reachable && completes,
                        Some(&usize_ty),
                    )?;
                    let index_flow = self.flow(index);
                    self.require_index(&index_flow)?;
                    completes &= index_flow.completes;
                    let saved_index = self.temporary(
                        &format!("$ni{}", self.locals.len()),
                        usize_ty.clone(),
                        false,
                    )?;
                    self.charge_node(std::mem::size_of::<ir::TypedStmt>() * 2)?;
                    statements.push(ir::TypedStmt::Let {
                        local: saved_index,
                        value: index,
                    });
                    let probe_index =
                        self.for_expr(usize_ty.clone(), ir::ExprKind::Local(saved_index))?;
                    self.index_bounds_probe(&place, probe_index, original, statements)?;
                    if position == last {
                        final_index = Some(
                            self.for_expr(usize_ty.clone(), ir::ExprKind::Local(saved_index))?,
                        );
                        break;
                    }
                    let ir::Ty::Array {
                        element: row_ty, ..
                    } = &place_ty
                    else {
                        return Err("runtime index assignment requires an array".into());
                    };
                    let row_ty = row_ty.as_ref().clone();
                    place
                        .projection
                        .push(ir::Projection::DynamicIndex(saved_index));
                    place_ty = row_ty;
                }
            }
        }
        let loaded = self.for_expr(saved_ty, ir::ExprKind::Local(saved))?;
        self.charge_node(std::mem::size_of::<ir::TypedStmt>())?;
        Ok((
            match (final_index, op) {
                (Some(index), Some(op)) => ir::TypedStmt::AssignIndexOp {
                    array: place,
                    index,
                    op,
                    value: loaded,
                },
                (Some(index), None) => ir::TypedStmt::AssignIndex {
                    array: place,
                    index,
                    value: loaded,
                },
                (None, Some(op)) => ir::TypedStmt::AssignOp {
                    place,
                    op,
                    value: loaded,
                },
                (None, None) => ir::TypedStmt::AssignPlace {
                    place,
                    value: loaded,
                },
            },
            completes,
        ))
    }
    /// A nonconsuming read expression of a typed local-rooted place, built
    /// from Local / Field / Index / DynamicIndex(saved local) expressions.
    pub(super) fn place_read_expr(
        &mut self,
        place: &ir::Place,
        ty: ir::Ty,
    ) -> Result<ir::ExprId, String> {
        let mut current_ty = self.locals[place.local.0].ty.clone();
        let mut id = self.for_expr(current_ty.clone(), ir::ExprKind::Local(place.local))?;
        for projection in &place.projection {
            let (kind, next) = match projection {
                ir::Projection::Field(index) => (
                    ir::ExprKind::Field {
                        base: id,
                        index: *index,
                    },
                    current_ty
                        .field_ty(*index)
                        .ok_or("invalid field projection")?
                        .clone(),
                ),
                ir::Projection::ConstantIndex(index) => (
                    ir::ExprKind::Index {
                        base: id,
                        index: *index,
                    },
                    self.index_type(&current_ty, *index)?,
                ),
                ir::Projection::DynamicIndex(local) => {
                    let ir::Ty::Array { element, .. } = &current_ty else {
                        return Err("runtime index assignment requires an array".into());
                    };
                    let element = element.as_ref().clone();
                    let index =
                        self.for_expr(ir::Ty::Int(ir::IntTy::Usize), ir::ExprKind::Local(*local))?;
                    (ir::ExprKind::DynamicIndex { base: id, index }, element)
                }
                _ => return Err("contract: nested index place projection".into()),
            };
            id = self.for_expr(next.clone(), kind)?;
            current_ty = next;
        }
        if current_ty != ty {
            return Err("contract: nested index place type".into());
        }
        Ok(id)
    }
    /// Explicit stop after any recorded planning resource failure. PlanBudget
    /// records the first failure but still admits later in-limit charges, so
    /// producers must not allocate or publish once a failure exists.
    fn plan_budget_guard(&self) -> Result<(), String> {
        if let Some(plan) = &self.signatures.plan
            && plan.budget.has_resource_failure()
        {
            return Err("planning resource budget already exhausted".into());
        }
        Ok(())
    }
    /// Paid projection for a place rooted at a call-returned reference. Only
    /// `deref_assignment` uses it; `checked_place` is unchanged. Pass A walks
    /// selectors without allocating and counts projections; the exact count
    /// is then allocated through `retained_pattern_vec` (charged before
    /// allocation, with any capacity surplus charged before return); pass B
    /// fills that buffer without growth. The hidden carrier's binding
    /// mutability is never authority: writability comes only from the
    /// reference kinds traversed.
    pub(super) fn call_place_projection(
        &self,
        carrier: &ir::Ty,
        fields: &[FieldSelector],
    ) -> Result<(Vec<ir::Projection>, ir::Ty, bool), String> {
        self.plan_budget_guard()?;
        if fields.len() > 128 {
            return Err("place projection nesting exceeds limit128".into());
        }
        let work = fields
            .len()
            .checked_add(1)
            .ok_or("place projection work overflow")?;
        self.signatures.plan()?.budget.charge_work(work)?;
        let (requested, _, _) = self.walk_call_place(carrier, fields, None)?;
        self.plan_budget_guard()?;
        let mut projection = self.retained_pattern_vec::<ir::Projection>(requested)?;
        self.plan_budget_guard()?;
        self.signatures.plan()?.budget.charge_work(work)?;
        let (pushed, destination, writable) =
            self.walk_call_place(carrier, fields, Some(&mut projection))?;
        if pushed != requested || projection.len() != requested {
            return Err("call place projection count mismatch".into());
        }
        self.plan_budget_guard()?;
        Ok((projection, destination, writable))
    }
    /// One selector walk with `checked_place` rule order. With `out == None`
    /// it only counts; with a buffer it pushes without ever growing it.
    fn walk_call_place(
        &self,
        carrier: &ir::Ty,
        fields: &[FieldSelector],
        mut out: Option<&mut Vec<ir::Projection>>,
    ) -> Result<(usize, ir::Ty, bool), String> {
        let mut count = 0usize;
        let mut emit = |projection: ir::Projection| -> Result<(), String> {
            if let Some(out) = out.as_deref_mut() {
                if out.len() >= out.capacity() {
                    return Err("call place projection capacity exceeded".into());
                }
                out.push(projection);
            }
            count = count
                .checked_add(1)
                .ok_or("call place projection count overflow")?;
            Ok(())
        };
        let mut ty = carrier.clone();
        let mut writable = false;
        let mut derefs = 0;
        for selector in fields {
            match selector {
                FieldSelector::Deref => {
                    derefs += 1;
                    if derefs > 128 {
                        return Err("reference dereference depth exceeds limit128".into());
                    }
                    if let ir::Ty::RawPtr { kind, pointee } = &ty {
                        if self.unsafe_depth == 0 {
                            return Err("E0133: dereference of raw pointer is unsafe and requires unsafe block".into());
                        }
                        writable = *kind == ir::BorrowKind::Mut;
                        ty = pointee.as_ref().clone();
                        emit(ir::Projection::Deref)?;
                        continue;
                    }
                    let ir::Ty::Ref { kind, referent } = &ty else {
                        return Err("dereference requires a reference".into());
                    };
                    writable = *kind == ir::BorrowKind::Mut;
                    ty = referent.as_ref().clone();
                    emit(ir::Projection::Deref)?;
                }
                FieldSelector::ConstantIndex(index) => {
                    ty = self.index_type(&ty, *index)?;
                    emit(ir::Projection::ConstantIndex(*index))?;
                }
                _ => {
                    if let ir::Ty::Ref { kind, referent } = &ty {
                        if derefs >= 128 {
                            return Err(
                                "multiple dereferences in one field place are unsupported".into()
                            );
                        }
                        if fields.len() >= 128 {
                            return Err(
                                "normalized place projection nesting exceeds limit128".into()
                            );
                        }
                        derefs += 1;
                        writable = *kind == ir::BorrowKind::Mut;
                        ty = referent.as_ref().clone();
                        emit(ir::Projection::Deref)?;
                    }
                    let (index, field) = self.field_type(&ty, selector)?;
                    ty = field;
                    emit(ir::Projection::Field(index))?;
                }
            }
        }
        self.accessible(&ty)?;
        Ok((count, ty, writable))
    }
    pub(super) fn block_when(
        &mut self,
        b: &Block,
        reachable: bool,
    ) -> Result<(ir::TypedBlock, Flow), String> {
        self.block_expected_when(b, reachable, None)
    }
    pub(super) fn block_expected_when(
        &mut self,
        b: &Block,
        reachable: bool,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::TypedBlock, Flow), String> {
        self.block_with_expectation_when(b, reachable, expected.into())
    }
    fn block_with_expectation_when(
        &mut self,
        b: &Block,
        reachable: bool,
        expectation: Expectation<'_>,
    ) -> Result<(ir::TypedBlock, Flow), String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.block_with_expectation(b, expectation);
        self.reachable = previous;
        result
    }
    pub(super) fn block(&mut self, b: &Block) -> Result<(ir::TypedBlock, Flow), String> {
        self.block_expected(b, None)
    }
    pub(super) fn block_expected(
        &mut self,
        b: &Block,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::TypedBlock, Flow), String> {
        self.block_with_expectation(b, expected.into())
    }
    pub(super) fn block_with_expectation(
        &mut self,
        b: &Block,
        expectation: Expectation<'_>,
    ) -> Result<(ir::TypedBlock, Flow), String> {
        let hints = if let Some(inputs) = self.nominal_inputs {
            match local_literals::hints_with_inputs(self, b, expectation.hard(), inputs)? {
                NominalPreparedHints::Ready(hints) => hints,
                NominalPreparedHints::Blocked(blocked) => {
                    *self.nominal_blocked.borrow_mut() = Some(blocked);
                    return Err(
                        "missing graph input: nested nominal hint preparation blocked".into(),
                    );
                }
            }
        } else {
            local_literals::hints(self, b, expectation.hard())?
        };
        self.block_with_resolved_hints(b, expectation, hints)
    }

    fn block_with_resolved_hints(
        &mut self,
        b: &Block,
        expectation: Expectation<'_>,
        hints: local_literals::Hints,
    ) -> Result<(ir::TypedBlock, Flow), String> {
        self.closure_signature_hints.extend(hints.closures);
        let literal_hints = hints.locals;
        let entered_reachable = self.reachable;
        let pending_start = self.pending_references.len();
        let temporaries_start = self.borrow_temporaries.len();
        self.scopes.push(HashMap::new());
        let mut live = true;
        // While-let remains a source Unit obligation even when its genuine
        // scrutinee cannot complete; keep runtime reachability separate.
        let mut source_live = true;
        let mut statements = Vec::with_capacity(b.statements.len());
        for s in &b.statements {
            self.reachable = entered_reachable && live;
            let (statement, completes) = match s {
                _ if let Some(result) = self.overloaded_index_assignment(s, &mut statements)? => {
                    result
                }
                Stmt::Declare { name, ty, mutable } => {
                    if matches!(ty, Type::Infer) {
                        let BindingName::Named(name) = name else {
                            return Err("untyped wildcard declarations are unsupported".into());
                        };
                        let key = self.local_key(name, true)?.ok_or("missing local key")?;
                        self.charge_node(std::mem::size_of::<PendingReferenceDecl>() * 4)?;
                        self.scopes.last_mut().unwrap().remove(&key);
                        self.pending_references.push(PendingReferenceDecl {
                            name: *name,
                            mutable: *mutable,
                            scope: self.scopes.len() - 1,
                            position: statements.len(),
                            local: None,
                        });
                        continue;
                    }
                    if matches!(ty, Type::Never) {
                        return Err(
                            "deferred declarations require a concrete supported source type".into(),
                        );
                    }
                    let ty = if partial_annotation::has_nested_infer(ty) {
                        let inferred = literal_hints
                            .get(&(s as *const Stmt as usize))
                            .cloned()
                            .ok_or("E0282: deferred type annotation remains unresolved")?;
                        partial_annotation::matches_partial(ty, &inferred, self.signatures)?;
                        inferred
                    } else {
                        self.source_local_ty(ty)?
                    };
                    self.accessible(&ty)?;
                    let local = self.local(name, ty, *mutable)?;
                    if let Some(cap) = self.canonical {
                        cap.record_binding(s, local)?;
                    }
                    (ir::TypedStmt::Declare { local }, true)
                }
                Stmt::Let {
                    name,
                    mutable,
                    ty,
                    value,
                } => {
                    let partial = partial_annotation::has_nested_infer(ty);
                    let written = ty;
                    let ty = if partial { &Type::Infer } else { ty };
                    let annotation = if matches!(ty, Type::Infer) {
                        literal_hints.get(&(s as *const Stmt as usize)).cloned()
                    } else {
                        Some(self.source_local_ty(ty)?)
                    };
                    let extending = self.extending_borrows.len();
                    self.mark_extending_borrows(value, 0)?;
                    let expected = if matches!(written, Type::Infer) {
                        annotation
                            .as_ref()
                            .map_or(Expectation::None, Expectation::Inferred)
                    } else {
                        annotation.as_ref().into()
                    };
                    let value = self.expr_with_expectation(value, expected);
                    self.extending_borrows.truncate(extending);
                    let value = value?;
                    let f = self.flow(value);
                    let t = if matches!(ty, Type::Infer) {
                        f.ty.clone()
                    } else {
                        let ty = self.source_local_ty(ty)?;
                        self.accessible(&ty)?;
                        self.require(&f, &ty)?;
                        ty
                    };
                    if partial {
                        partial_annotation::matches_partial(written, &t, self.signatures)?;
                    }
                    let local = self.local(name, t, *mutable)?;
                    if let Some(cap) = self.canonical {
                        cap.record_binding(s, local)?;
                    }
                    (ir::TypedStmt::Let { local, value }, f.completes)
                }
                Stmt::LetElse {
                    pattern,
                    ty,
                    value,
                    else_block,
                } => self.let_else_statement(pattern, ty, value, else_block)?,
                Stmt::LetPattern { pattern, ty, value } => {
                    let partial = partial_annotation::has_nested_infer(ty);
                    let written = ty;
                    let ty = if partial { &Type::Infer } else { ty };
                    // Resolve/check every initializer against the complete old scope.
                    let annotation = if matches!(ty, Type::Infer) {
                        None
                    } else {
                        Some(self.source_local_ty(ty)?)
                    };
                    // An unannotated pattern let still has a solved initializer
                    // type. Use it only as an inferred expectation, never as a
                    // hard annotation, exactly like the plain-let path.
                    let inferred = if matches!(written, Type::Infer) {
                        literal_hints.get(&(s as *const Stmt as usize)).cloned()
                    } else {
                        None
                    };
                    let direct_dynamic = matches!(value, Expr::DynamicIndex { .. });
                    let direct_wildcard = matches!(pattern, Pattern::Wildcard);
                    let extending = self.extending_borrows.len();
                    self.mark_extending_borrows(value, 0)?;
                    let value = match (&annotation, &inferred) {
                        (Some(ty), _) => self.expr_expected(value, Some(ty)),
                        (None, Some(hint)) => {
                            self.expr_with_expectation(value, Expectation::Inferred(hint))
                        }
                        (None, None) => self.expr_expected(value, None),
                    };
                    self.extending_borrows.truncate(extending);
                    let value = value?;
                    let flow = self.flow(value);
                    if direct_dynamic && !self.facts().must_copy(&flow.ty)? && !direct_wildcard {
                        return Err("non-Copy runtime indexed pattern extraction is unsupported; only direct let _ inspection is admitted".into());
                    }
                    let seed = if matches!(ty, Type::Infer) {
                        flow.ty.clone()
                    } else {
                        let ty = self.source_local_ty(ty)?;
                        self.accessible(&ty)?;
                        self.require(&flow, &ty)?;
                        ty
                    };
                    if partial {
                        partial_annotation::matches_partial(written, &seed, self.signatures)?;
                    }
                    let ty = self.pattern_ty(pattern, &seed)?;
                    self.require(&flow, &ty)?;
                    self.accessible(&ty)?;
                    let mut pending = Vec::new();
                    self.pattern_locals(
                        pattern,
                        &ty,
                        &mut Vec::new(),
                        &mut std::collections::HashSet::new(),
                        &mut pending,
                        &mut 0usize,
                    )?;
                    // No partially installed binder set can affect another binder.
                    let mut bindings = Vec::with_capacity(pending.len());
                    for binding in pending {
                        let local = self.local(binding.name, binding.ty, binding.mutable)?;
                        bindings.push(ir::PatternBinding {
                            local,
                            projection: binding.projection,
                            borrow: binding.borrow,
                        });
                    }
                    (
                        ir::TypedStmt::LetPattern {
                            value,
                            ty,
                            bindings,
                        },
                        flow.completes,
                    )
                }
                Stmt::AssignNestedIndex { target, op, value } => {
                    self.nested_index_assignment(target, *op, value, &mut statements)?
                }
                Stmt::AssignDeref {
                    carrier,
                    fields,
                    index,
                    op,
                    value,
                    ..
                } => self.deref_assignment(
                    carrier,
                    fields,
                    index.as_ref(),
                    *op,
                    value,
                    &mut statements,
                )?,
                Stmt::Assign { name, value } => {
                    if let Some(pending) = self.pending_reference(name)? {
                        // Only the already known dereference result supplies a scalar
                        // type here. Later uses never contribute inference evidence.
                        let scalar = if matches!(value, Expr::Deref { .. }) {
                            self.scalar_hint(value)?.filter(|ty| {
                                matches!(
                                    ty,
                                    ir::Ty::Int(_)
                                        | ir::Ty::Float(_)
                                        | ir::Ty::Char
                                        | ir::Ty::Bool
                                        | ir::Ty::Unit
                                )
                            })
                        } else {
                            None
                        };
                        let value = self.expr(value)?;
                        let flow = self.flow(value);
                        if !matches!(flow.ty, ir::Ty::Ref { .. })
                            && scalar.as_ref() != Some(&flow.ty)
                        {
                            return Err("untyped deferred declarations require an independently known reference or scalar dereference type; general backward inference is unsupported".into());
                        }
                        let local = ir::LocalId(self.locals.len());
                        let key = self
                            .local_key(name, false)?
                            .ok_or("missing assignment local key")?;
                        let text = name.diagnostic(self.names()?)?;
                        self.charge_node(
                            std::mem::size_of::<ir::LocalDecl>() + text.len() * 2 + 128,
                        )?;
                        if self.lexical_scope.is_some() {
                            let scope = self.pending_references[pending].scope;
                            self.reserve_scoped_local(scope)?;
                        }
                        let declaration = &mut self.pending_references[pending];
                        declaration.local = Some(local);
                        self.locals.push(ir::LocalDecl {
                            name: text,
                            ty: flow.ty,
                            mutable: declaration.mutable,
                        });
                        self.scopes[declaration.scope].insert(key, local);
                        let statement = self.scope_borrow_temporaries(
                            temporaries_start,
                            ir::TypedStmt::Assign { local, value },
                            flow.completes,
                            &mut statements,
                        )?;
                        statements.push(statement);
                        live &= flow.completes;
                        continue;
                    }
                    let local = self.lookup(name)?;
                    let declaration = &self.locals[local.0];
                    let t = declaration.ty.clone();
                    let value = self.expr_expected(value, Some(&t))?;
                    let f = self.flow(value);
                    self.require(&f, &t)?;
                    (ir::TypedStmt::Assign { local, value }, f.completes)
                }
                Stmt::AssignIndex {
                    array,
                    index,
                    value,
                    ..
                } if let Some(done) =
                    self.mut_slice_index_assignment(array, index, None, value)? =>
                {
                    done
                }
                Stmt::AssignIndex {
                    array,
                    index,
                    value,
                    ..
                } => {
                    let (array, ty, writable) = self.checked_place(array)?;
                    if array.projection.contains(&ir::Projection::Deref) {
                        return Err("runtime indexed access through references is unsupported in B1; snapshot a Copy referent first".into());
                    }
                    if !writable {
                        return Err("runtime index assignment requires a mutable place".into());
                    }
                    let ir::Ty::Array { element, .. } = &ty else {
                        return Err("runtime index assignment requires an array".into());
                    };
                    let value = self.expr_expected(value, Some(element))?;
                    let flow = self.flow(value);
                    self.require(&flow, element)?;
                    let index = self.expr_expected_when(
                        index,
                        self.reachable && flow.completes,
                        Some(&ir::Ty::Int(ir::IntTy::Usize)),
                    )?;
                    let index_flow = self.flow(index);
                    self.require_index(&index_flow)?;
                    (
                        ir::TypedStmt::AssignIndex {
                            array,
                            index,
                            value,
                        },
                        flow.completes && index_flow.completes,
                    )
                }
                Stmt::AssignOp {
                    place, op, value, ..
                } if let Some(done) =
                    self.mut_slice_constant_assignment(place, Some(*op), value)? =>
                {
                    done
                }
                Stmt::AssignOp {
                    place, op, value, ..
                } if let Some(id) = self.overloaded_assign(s, place, *op, value)? => {
                    (ir::TypedStmt::Expr(id), self.flow(id).completes)
                }
                Stmt::AssignOp {
                    place, op, value, ..
                } => {
                    let (place, ty, _) = self.checked_place(place)?;
                    let (value, flow) = self.compound_value(*op, &ty, value)?;
                    (
                        ir::TypedStmt::AssignOp {
                            place,
                            op: *op,
                            value,
                        },
                        flow.completes,
                    )
                }
                Stmt::AssignIndexOp {
                    array,
                    index,
                    op,
                    value,
                    ..
                } if let Some(done) =
                    self.mut_slice_index_assignment(array, index, Some(*op), value)? =>
                {
                    done
                }
                Stmt::AssignIndexOp {
                    array,
                    index,
                    op,
                    value,
                    ..
                } => {
                    let (mut array, mut ty, mut writable) = self.checked_place(array)?;
                    if let ir::Ty::Ref { kind, referent } = &ty {
                        if array.projection.len() >= 128 {
                            return Err(
                                "normalized place projection nesting exceeds limit128".into()
                            );
                        }
                        writable = *kind == ir::BorrowKind::Mut;
                        ty = referent.as_ref().clone();
                        array.projection.push(ir::Projection::Deref);
                    }
                    if !writable {
                        return Err("compound index assignment requires a mutable place; shared references cannot write".into());
                    }
                    let ir::Ty::Array { element, .. } = ty else {
                        return Err("compound index assignment requires an array".into());
                    };
                    // Retain the place rather than reading/copying the referent.
                    // Lowering snapshots RHS, then index, then resolves this
                    // current carrier and performs the checked read/write.
                    let (value, flow) = self.compound_value(*op, &element, value)?;
                    let index = self.expr_expected_when(
                        index,
                        self.reachable && flow.completes,
                        Some(&ir::Ty::Int(ir::IntTy::Usize)),
                    )?;
                    let index_flow = self.flow(index);
                    self.require_index(&index_flow)?;
                    (
                        ir::TypedStmt::AssignIndexOp {
                            array,
                            index,
                            op: *op,
                            value,
                        },
                        flow.completes && index_flow.completes,
                    )
                }
                Stmt::AssignPlace { place, value, .. }
                    if let Some(done) =
                        self.mut_slice_constant_assignment(place, None, value)? =>
                {
                    done
                }
                Stmt::AssignPlace { place, value, .. } => {
                    let (place, ty, writable) = self.checked_place(place)?;
                    if !place.projection.is_empty()
                        && !writable
                        && !place.projection.contains(&ir::Projection::Deref)
                    {
                        return Err(
                            "assignment requires a mutable place; shared references cannot write"
                                .into(),
                        );
                    }
                    let value = self.expr_expected(value, Some(&ty))?;
                    let flow = self.flow(value);
                    self.require(&flow, &ty)?;
                    (ir::TypedStmt::AssignPlace { place, value }, flow.completes)
                }
                Stmt::Expr(e) | Stmt::ImplicitExpr(e) => {
                    let value = self.expr(e)?;
                    let f = self.flow(value);
                    if matches!(s, Stmt::ImplicitExpr(_)) {
                        self.require(&f, &ir::Ty::Unit)?;
                    }
                    (ir::TypedStmt::Expr(value), f.completes)
                }
                Stmt::Return(e) => {
                    let result_ty = self.result.clone();
                    let value = e
                        .as_ref()
                        .map(|e| {
                            let expectation = if self.inferred_closure_result {
                                Expectation::Inferred(&result_ty)
                            } else {
                                Expectation::HasType(&result_ty)
                            };
                            self.expr_with_expectation(e, expectation)
                        })
                        .transpose()?;
                    let f = value.map_or(Flow::value(ir::Ty::Unit), |e| self.flow(e));
                    self.require(&f, &self.result)?;
                    (ir::TypedStmt::Return(value), false)
                }
                Stmt::While {
                    label,
                    condition,
                    body,
                } => {
                    // Never skip a condition's target barrier to an outer loop.
                    let id = self.enter_loop(false);
                    let key = self.label_key(*label)?;
                    self.loops.last_mut().unwrap().label = key;
                    let condition = self.expr(condition)?;
                    let f = self.flow(condition);
                    self.require(&f, &ir::Ty::Bool)?;
                    self.loops.last_mut().unwrap().checking_condition = false;
                    let (body, bf) = self.block(body)?;
                    self.require(&bf, &ir::Ty::Unit)?;
                    self.loops.pop();
                    (
                        ir::TypedStmt::While {
                            id,
                            condition,
                            body,
                        },
                        f.completes,
                    )
                }
                Stmt::For {
                    label,
                    pattern,
                    source,
                    body,
                    ..
                } => self.for_range_statement(
                    *label,
                    pattern,
                    source,
                    body,
                    literal_hints.get(&(s as *const Stmt as usize)),
                )?,
                Stmt::WhileLet {
                    label,
                    pattern,
                    scrutinee,
                    body,
                    ..
                } => self.while_let_statement(*label, pattern, scrutinee, body)?,
            };
            let statement = match s {
                Stmt::AssignIndex {
                    site: Some(site), ..
                }
                | Stmt::AssignIndexOp {
                    site: Some(site), ..
                }
                | Stmt::AssignDeref {
                    site: Some(site), ..
                }
                | Stmt::AssignPlace {
                    site: Some(site), ..
                }
                | Stmt::AssignOp {
                    site: Some(site), ..
                } => self.assignment_bounds_probe(statement, *site, &mut statements)?,
                _ => statement,
            };
            let statement = self.scope_borrow_temporaries(
                temporaries_start,
                statement,
                completes,
                &mut statements,
            )?;
            if let Some(cap) = self.canonical {
                cap.record_emission(s, statements.len())?;
            }
            statements.push(statement);
            live &= completes;
            if !matches!(s, Stmt::WhileLet { .. } | Stmt::For { .. }) {
                source_live &= completes;
            }
        }
        self.reachable = entered_reachable && live;
        let tail = b
            .tail
            .as_ref()
            .map(|e| self.expr_with_expectation(e, expectation))
            .transpose()?;
        self.reachable = entered_reachable;
        // Tail temporaries live until the end of this block.
        for (local, _) in self.borrow_temporaries.drain(temporaries_start..) {
            statements.push(ir::TypedStmt::Declare { local });
        }
        let mut flow = tail.map_or_else(
            || {
                // An undiscarded genuine final while-let is a Unit-valued
                // source tail even after a preceding divergent statement.
                if source_live
                    || matches!(
                        b.statements.last(),
                        Some(
                            Stmt::WhileLet {
                                has_semicolon: false,
                                ..
                            } | Stmt::For {
                                has_semicolon: false,
                                ..
                            }
                        )
                    )
                {
                    Flow::value(ir::Ty::Unit)
                } else {
                    Flow::never()
                }
            },
            |e| self.flow(e),
        );
        flow.completes &= live;
        let count = self.pending_references.len() - pending_start;
        if let Some(plan) = &self.signatures.plan {
            plan.budget.charge_work(
                count
                    .checked_mul(statements.len().saturating_add(count))
                    .ok_or("pending declaration insertion work overflow")?,
            )?;
        }
        self.charge_node(count.saturating_mul(std::mem::size_of::<ir::TypedStmt>() * 4))?;
        for (offset, pending) in self.pending_references.drain(pending_start..).enumerate() {
            let local = pending
                .local
                .ok_or("untyped deferred reference declaration remains underconstrained")?;
            let position = pending
                .position
                .checked_add(offset)
                .ok_or("pending declaration position overflow")?;
            if let Some(cap) = self.canonical {
                cap.shift_root_emissions(b, position)?;
            }
            statements.insert(position, ir::TypedStmt::Declare { local });
        }
        self.scopes.pop();
        Ok((ir::TypedBlock { statements, tail }, flow))
    }
    pub(super) fn expr(&mut self, e: &Expr) -> Result<ir::ExprId, String> {
        self.expr_expected(e, None)
    }
    pub(super) fn expr_expected(
        &mut self,
        e: &Expr,
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        self.expr_with_expectation(e, expected.into())
    }
    pub(super) fn expr_with_expectation(
        &mut self,
        e: &Expr,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        self.prepare_callable_arguments(e, 0)?;
        if let Some(ir::Ty::Opaque(opaque)) = expectation.coercion() {
            let opaque = opaque.clone();
            if self.declaration == Some(opaque.owner()) {
                let value = self.expr_with_expectation(e, Expectation::None)?;
                return self.coerce_opaque_return(value, &opaque);
            }
        }
        if let Expr::Var(path) = e {
            let key = self.path_key(path, Namespace::Value)?;
            let signature = self.closure_signature_hints.get(&key).cloned();
            if let Some(value) = self.check_stored_closure(path, signature.as_ref())? {
                return Ok(value);
            }
        }
        if let Some(id) = self.builtin_leading_zeros(e)? {
            if let Some(cap) = self.canonical {
                cap.record_builtin_leading_zeros(e, id, &self.exprs)?;
            }
            return Ok(id);
        }
        if let Some(id) = self.builtin_slice_len(e)? {
            if let Some(cap) = self.canonical {
                cap.record_builtin_len(e, id, &self.exprs)?;
            }
            return Ok(id);
        }
        if let Some(id) = self.builtin_intrinsic(e)? {
            if let Some(cap) = self.canonical {
                cap.record_builtin_intrinsic(e, id, &self.exprs)?;
            }
            return Ok(id);
        }
        let result = self.expr_with_expectation_inner(e, expectation)?;
        self.record_runtime_site(e, result)?;
        let result = self.record_original_call(e, result)?;
        match expectation.coercion() {
            Some(expected)
                if expected.slice_ref_element().is_some_and(|(kind, _)| {
                    self.exprs[result.0]
                        .ty
                        .array_ref_element_len()
                        .is_some_and(|(from, _, _)| from == kind)
                }) =>
            {
                self.coerce_shared_array_to_slice(result, expected)
            }
            Some(expected) if trait_object_target(expected).is_some() => {
                self.coerce_ref_to_dyn(result, expected)
            }
            Some(expected) => Ok(self.deref_coercion(e, result, expected)?.unwrap_or(result)),
            _ => Ok(result),
        }
    }
    /// Expected-site unsizing `&T`/`&mut T` (or reborrowing `&mut T` as `&T`)
    /// to `&dyn P`/`&mut dyn P`. Requires a proof of `T: P` in the current
    /// facts (E0277 otherwise). Already-`dyn` operands pass through unchanged.
    pub(super) fn coerce_ref_to_dyn(
        &mut self,
        value: ir::ExprId,
        expected: &ir::Ty,
    ) -> Result<ir::ExprId, String> {
        let (want_kind, object) =
            trait_object_target(expected).ok_or("trait object coercion target")?;
        let principal = object.principal;
        let source = &self.exprs[value.0];
        let ir::Ty::Ref {
            kind: have_kind,
            referent,
        } = &source.ty
        else {
            return Ok(value);
        };
        if matches!(referent.as_ref(), ir::Ty::Dyn(_)) {
            return Ok(value);
        }
        if matches!(source.ty, ir::Ty::Never) {
            return Ok(value);
        }
        if matches!(
            referent.as_ref(),
            ir::Ty::Slice { .. } | ir::Ty::Never | ir::Ty::AssociatedType(_)
        ) {
            return Err("unsupported: trait object unsizing from this referent type".into());
        }
        if want_kind == ir::BorrowKind::Mut && *have_kind == ir::BorrowKind::Shared {
            // Leave the mismatch to ordinary checking (E0308).
            return Ok(value);
        }
        if want_kind != *have_kind {
            return Err(
                "unsupported: coercing `&mut T` to `&dyn Trait` (reborrow then unsize)".into(),
            );
        }
        let referent = referent.as_ref().clone();
        let facts = self
            .signatures
            .facts
            .as_deref()
            .ok_or("trait object coercion requires validated facts")?;
        let goal = ir::Predicate::Implements(ir::TraitRef {
            declaration: principal,
            self_ty: referent.clone(),
            args: object.args.clone(),
        });
        match facts
            .service()
            .prove_goal(&goal, facts)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
        {
            super::super::traits::ProofOutcome::Proved(_) => {}
            super::super::traits::ProofOutcome::NotProven => {
                return Err(format!(
                    "E0277: the trait bound `{referent:?}: {}` is not satisfied (trait object unsizing)",
                    self.signatures.plan()?.declaration(principal)?.name
                ));
            }
        }
        facts
            .prove_dynamic_unsize(&referent, &object)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
        let completion = source.completion;
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: expected.clone(),
            completion,
            kind: ir::ExprKind::RefToDyn { reference: value },
        });
        if let Some(cap) = self.canonical {
            cap.record_unsize(id, value)?;
        }
        Ok(id)
    }
    /// Expected-site unsizing of shared `&[T; N]` to exactly shared `&[T]`.
    /// The operand keeps its own identity (including any recorded original call);
    /// the wrapper is recorded separately with its exact operand for replay.
    /// Shared subslice `&base[start..end]` (SUBSLICE.md). The base must be a
    /// shared `&[T]` or a shared reference to / local of an array, which is
    /// first unsized. Evaluates base, start, end once in source order.
    pub(super) fn subslice(
        &mut self,
        base: &Expr,
        start: Option<&Expr>,
        end: Option<&Expr>,
        inclusive: bool,
    ) -> Result<ir::ExprId, String> {
        self.subslice_with_kind(ir::BorrowKind::Shared, base, start, end, inclusive)
    }

    pub(super) fn subslice_with_kind(
        &mut self,
        kind: ir::BorrowKind,
        base: &Expr,
        start: Option<&Expr>,
        end: Option<&Expr>,
        inclusive: bool,
    ) -> Result<ir::ExprId, String> {
        let usize_ty = ir::Ty::Int(ir::IntTy::Usize);
        let mut slice = self.range_slice_base(kind, base)?;
        let mut flow = self.flow(slice);
        if let Some((source_kind, element, _)) = flow.ty.array_ref_element_len() {
            let target = ir::Ty::Ref {
                kind: source_kind,
                referent: std::sync::Arc::new(ir::Ty::Slice {
                    element: std::sync::Arc::new(element.clone()),
                }),
            };
            slice = self.coerce_shared_array_to_slice(slice, &target)?;
            flow = self.flow(slice);
        }
        if flow.ty.is_shared_str() {
            return Err("unsupported: str range slicing".into());
        }
        let (source_kind, element) = flow
            .ty
            .slice_ref_element()
            .ok_or("unsupported: range indexing requires a slice or array")?;
        if source_kind != kind {
            return Err("E0596: range borrow requires matching source mutability".into());
        }
        let element = element.clone();
        ir::validate_shared_slice_element(&element, self.facts(), &self.signatures.plan()?.budget)?;
        let mut completes = flow.completes;
        let mut bound =
            |checker: &mut Self, value: Option<&Expr>| -> Result<Option<ir::ExprId>, String> {
                let Some(value) = value else { return Ok(None) };
                let id = checker.expr_expected_when(
                    value,
                    checker.reachable && completes,
                    Some(&usize_ty),
                )?;
                let f = checker.flow(id);
                checker
                    .require_index(&f)
                    .map_err(|_| "E0277: range index bound requires usize".to_string())?;
                completes &= f.completes;
                Ok(Some(id))
            };
        let start = bound(self, start)?;
        let end = bound(self, end)?;
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: flow.ty.clone(),
            completion: if completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind: ir::ExprKind::Subslice {
                slice,
                start,
                end,
                inclusive,
            },
        });
        Ok(id)
    }
    pub(super) fn coerce_shared_array_to_slice(
        &mut self,
        value: ir::ExprId,
        expected: &ir::Ty,
    ) -> Result<ir::ExprId, String> {
        let (kind, element) = expected
            .slice_ref_element()
            .ok_or("array-to-slice coercion target must be a slice reference")?;
        let source = &self.exprs[value.0];
        let (from, actual, _) = source
            .ty
            .array_ref_element_len()
            .ok_or("array-to-slice coercion requires an array reference operand")?;
        if from != kind {
            return Err("array-to-slice coercion changed reference mutability".into());
        }
        let budget = &self.signatures.plan()?.budget;
        ir::validate_shared_slice_element(element, self.facts(), budget)?;
        ir::validate_shared_slice_element(actual, self.facts(), budget)?;
        if !metered_type_equal(actual, element, budget, 0)? {
            return Err("array-to-slice coercion element type mismatch".into());
        }
        let completion = source.completion;
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: expected.clone(),
            completion,
            kind: ir::ExprKind::ArrayToSlice { reference: value },
        });
        if let Some(cap) = self.canonical {
            cap.record_unsize(id, value)?;
        }
        Ok(id)
    }
    /// Admit only an exactly typed by-value integer and zero method arguments.
    /// Other receiver kinds and signatures keep ordinary method resolution.
    pub(super) fn builtin_leading_zeros(&mut self, e: &Expr) -> Result<Option<ir::ExprId>, String> {
        let Expr::MethodCall {
            receiver,
            name,
            type_args,
            args,
        } = e
        else {
            return Ok(None);
        };
        if !leading_zeros_candidate(e, self.names()?)?
            || !matches!(self.scalar_hint(receiver)?, Some(ir::Ty::Int(_)))
        {
            return Ok(None);
        }
        // The hint grants no type authority. Check and evaluate the receiver once.
        let value = self.expr(receiver)?;
        let signature = leading_zeros_signature(
            &self.exprs[value.0].ty,
            *name,
            type_args.len(),
            args.len(),
            self.names()?,
        )?
        .ok_or("leading_zeros receiver must be an exactly typed by-value integer")?;
        let completion = self.exprs[value.0].completion;
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: signature.result,
            kind: ir::ExprKind::LeadingZeros { value },
            completion,
        });
        Ok(Some(id))
    }

    /// Resolve `receiver.name(args)` to an inherent primitive method only when
    /// the receiver is independently known to be an exact scalar by value.
    /// Other receivers keep ordinary method resolution.
    fn intrinsic_target(&self, e: &Expr) -> Result<Option<ir::Intrinsic>, String> {
        let Expr::MethodCall {
            receiver,
            name,
            type_args,
            ..
        } = e
        else {
            return Ok(None);
        };
        if !type_args.is_empty() {
            return Ok(None);
        }
        let spelling = self
            .names()?
            .spelling(*name)
            .map_err(native_adapter::name_diagnostic)?;
        if !ir::IntrinsicOp::candidate_name(spelling) {
            return Ok(None);
        }
        // Method autoderef: `&T`/`&mut T` receivers (to bounded depth) reach
        // the scalar's by-value inherent method, as in stock resolution.
        let mut hint = self.scalar_hint(receiver)?;
        for _ in 0..MAX_INTRINSIC_AUTODEREF {
            match hint {
                Some(ir::Ty::Ref { referent, .. }) => hint = Some(referent.as_ref().clone()),
                _ => break,
            }
        }
        let Some(scalar) = hint.as_ref().and_then(ir::ScalarTy::of) else {
            return Ok(None);
        };
        Ok(ir::Intrinsic::resolve(scalar, spelling))
    }

    pub(super) fn builtin_intrinsic(&mut self, e: &Expr) -> Result<Option<ir::ExprId>, String> {
        let Some(intrinsic) = self.intrinsic_target(e)? else {
            return Ok(None);
        };
        let Expr::MethodCall { receiver, args, .. } = e else {
            return Ok(None);
        };
        if let Some(reason) = intrinsic.unsupported_reason() {
            return Err(format!("unsupported: {reason}"));
        }
        let signature = intrinsic
            .signature()
            .ok_or("primitive method signature invariant")?;
        if args.len() + 1 != signature.params.len() {
            return Err(format!(
                "E0061: primitive method takes {} argument(s) but {} were supplied",
                signature.params.len() - 1,
                args.len()
            ));
        }
        // The hint grants no type authority. Check the receiver, then each
        // argument once, in source order against the exact signature.
        let mut checked = Vec::with_capacity(signature.params.len());
        let mut value = self.expr(receiver)?;
        for _ in 0..MAX_INTRINSIC_AUTODEREF {
            let ir::Ty::Ref { referent, .. } = &self.exprs[value.0].ty else {
                break;
            };
            let ty = referent.as_ref().clone();
            let completion = self.exprs[value.0].completion;
            self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
            let id = ir::ExprId(self.exprs.len());
            self.exprs.push(ir::TypedExpr {
                ty,
                kind: ir::ExprKind::Deref { value },
                completion,
            });
            value = id;
        }
        if self.exprs[value.0].ty != signature.params[0] {
            return Err("primitive method receiver must be an exactly typed scalar".into());
        }
        checked.push(value);
        for (arg, param) in args.iter().zip(&signature.params[1..]) {
            let id = self.expr_expected(arg, Some(param))?;
            let flow = self.flow(id);
            self.require(&flow, param)?;
            checked.push(id);
        }
        let completion = if checked
            .iter()
            .all(|id| self.exprs[id.0].completion == ir::Completion::Returns)
        {
            ir::Completion::Returns
        } else {
            ir::Completion::Never
        };
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(
            std::mem::size_of::<ir::TypedExpr>()
                + checked.len() * std::mem::size_of::<ir::ExprId>(),
        )?;
        self.exprs.push(ir::TypedExpr {
            ty: signature.result,
            kind: ir::ExprKind::Intrinsic {
                intrinsic,
                args: checked,
            },
            completion,
        });
        Ok(Some(id))
    }

    /// Resolve genuine array/reference method candidates before the slice
    /// unsizing fallback. An in-scope array trait method can shadow slice len.
    fn array_len_shadowed_by_method(&self, e: &Expr) -> Result<bool, String> {
        let Expr::MethodCall {
            receiver,
            name,
            type_args,
            args,
        } = e
        else {
            return Ok(false);
        };
        let Some(ty) = self.method_receiver_hint(receiver)? else {
            return Ok(false);
        };
        if !array_len_receiver::receiver_is_array(&ty) {
            return Ok(false);
        }
        let (selection, deferred) =
            self.method_selection_with_inference_state(&ty, name, type_args, args, None)?;
        Ok(selection.is_some() || deferred)
    }

    /// Builtin `.len()` only on an exactly typed shared slice with no arguments.
    pub(super) fn builtin_slice_len(&mut self, e: &Expr) -> Result<Option<ir::ExprId>, String> {
        let Expr::MethodCall { receiver, .. } = e else {
            return Ok(None);
        };
        if !slice_len_candidate(e, self.names()?)?
            || !(matches!(receiver.as_ref(), Expr::Array(_) | Expr::Repeat { .. })
                || self.method_receiver_hint(receiver)?.is_some_and(|ty| {
                    ty.descriptor_element().is_some() || array_len_receiver::receiver_is_array(&ty)
                }))
            || self.array_len_shadowed_by_method(e)?
        {
            return Ok(None);
        }
        let slice = match receiver.as_ref() {
            receiver @ (Expr::Array(_) | Expr::Repeat { .. }) => {
                self.borrow_value(ir::BorrowKind::Shared, receiver, None)?
            }
            receiver
                if array_len_receiver::receiver_is_array(
                    &self
                        .method_receiver_hint(receiver)?
                        .ok_or("array receiver type disappeared")?,
                ) =>
            {
                self.array_len_receiver(receiver)?
                    .ok_or("array receiver adjustment disappeared")?
            }
            receiver
                if self
                    .scalar_hint(receiver)?
                    .is_some_and(|ty| ty.mut_slice_element().is_some()) =>
            {
                self.shared_slice_view(receiver)?
                    .ok_or("unsupported: mutable slice read requires a local-rooted carrier")?
            }
            Expr::RangeIndex {
                base,
                start,
                end,
                inclusive,
            } => self.subslice(base, start.as_deref(), end.as_deref(), *inclusive)?,
            receiver => self.expr(receiver)?,
        };
        let slice = self.array_reference_as_slice(slice)?;
        let flow = self.flow(slice);
        if !flow.ty.is_shared_str() {
            let element = flow
                .ty
                .shared_slice_element()
                .ok_or("builtin slice length receiver changed type")?;
            ir::validate_shared_slice_element(
                element,
                self.facts(),
                &self.signatures.plan()?.budget,
            )?;
        }
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: ir::Ty::Int(ir::IntTy::Usize),
            completion: if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind: ir::ExprKind::SliceLen { slice },
        });
        Ok(Some(id))
    }
    /// Check the closed borrowed-field capability before attempting a whole-element move.
    /// Repeated borrowed scans avoid a new projection allocation and charge every step.
    #[inline(never)]
    fn reject_noncopy_slice_field(&mut self, expression: &Expr) -> Result<(), String> {
        let Some(plan) = &self.signatures.plan else {
            return Ok(());
        };
        let budget = plan.budget.clone();
        let mut root = expression;
        let mut fields = 0usize;
        while let Expr::Field { base, .. } = root {
            budget.charge_work(1)?;
            fields += 1;
            root = base;
        }
        let (receiver, dynamic_index) = match root {
            Expr::Index { base, .. } => (base.as_ref(), None),
            Expr::DynamicIndex { base, index, .. } => (base.as_ref(), Some(index.as_ref())),
            _ => return Ok(()),
        };
        let Some(hint) = self.scalar_hint(receiver)? else {
            return Ok(());
        };
        let Some(element) = hint.shared_slice_element() else {
            return Ok(());
        };
        ir::validate_shared_slice_element(element, self.facts(), &budget)?;
        if self.facts().must_copy(element)? {
            return Ok(());
        }
        let slice = self.expr(receiver)?;
        let source = self.flow(slice);
        let mut ty = source
            .ty
            .shared_slice_element()
            .ok_or("slice field receiver changed type")?
            .clone();
        ir::validate_shared_slice_element(&ty, self.facts(), &budget)?;
        if let Some(index) = dynamic_index {
            let index = self.expr_expected_when(
                index,
                self.reachable && source.completes,
                Some(&ir::Ty::Int(ir::IntTy::Usize)),
            )?;
            self.require_index(&self.flow(index))?;
        }
        for remaining in (0..fields).rev() {
            let mut node = expression;
            for _ in 0..remaining {
                budget.charge_work(1)?;
                let Expr::Field { base, .. } = node else {
                    unreachable!()
                };
                node = base;
            }
            budget.charge_work(1)?;
            let Expr::Field { field, .. } = node else {
                unreachable!()
            };
            ty = self.field_type(&ty, field)?.1;
        }
        if !self.facts().must_copy(&ty)? {
            return Err("cannot move a non-Copy field out of a borrowed slice element".into());
        }
        Err(super::super::source::SourceError {
            kind: super::super::source::SourceErrorKind::Unsupported,
            primary: None,
            message: "borrowed field reads through non-Copy slice elements are unsupported".into(),
            _diagnostic_lease: None,
            location: None,
        }
        .to_string())
    }
    pub(super) fn slice_index(
        &mut self,
        slice: ir::ExprId,
        index: &Expr,
    ) -> Result<ir::ExprId, String> {
        let source = self.flow(slice);
        let index = self.expr_expected_when(
            index,
            self.reachable && source.completes,
            Some(&ir::Ty::Int(ir::IntTy::Usize)),
        )?;
        self.slice_index_checked(slice, index)
    }
    pub(super) fn slice_index_checked(
        &mut self,
        slice: ir::ExprId,
        index: ir::ExprId,
    ) -> Result<ir::ExprId, String> {
        let source = self.flow(slice);
        let index_flow = self.flow(index);
        self.require_index(&index_flow)?;
        let element = source
            .ty
            .shared_slice_element()
            .ok_or("slice index receiver must be a shared slice")?;
        ir::validate_shared_slice_element(element, self.facts(), &self.signatures.plan()?.budget)?;
        if !self.facts().must_copy(element)? {
            return Err("cannot move a non-Copy element out of a borrowed slice".into());
        }
        let element = element.clone();
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: element,
            completion: if source.completes && index_flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind: ir::ExprKind::SliceIndex { slice, index },
        });
        Ok(id)
    }
    pub(super) fn record_original_call(
        &mut self,
        e: &Expr,
        result: ir::ExprId,
    ) -> Result<ir::ExprId, String> {
        if matches!(
            e,
            Expr::Call { .. }
                | Expr::MethodCall { .. }
                | Expr::AssociatedCall { .. }
                | Expr::TraitCall { .. }
        ) && let Some(cap) = self.canonical
        {
            // Primitive rvalue receivers use the existing single-evaluation block
            // wrapper. The original method occurrence binds its tail call, not
            // the wrapper expression, while the wrapper remains in the IR.
            let call = match (e, &self.exprs[result.0].kind) {
                (Expr::MethodCall { .. }, ir::ExprKind::Block(block)) => block
                    .tail
                    .ok_or("canonical method wrapper lost its tail call")?,
                _ => result,
            };
            cap.record_call(e, call, &self.exprs[call.0].kind)?;
        }
        Ok(result)
    }
    pub(super) fn expr_with_expectation_inner(
        &mut self,
        e: &Expr,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        if let Expr::Apply { path, value } = e {
            let previous = self.applications.borrow().len();
            if let Some((key, args)) =
                self.partial_call_application(path, value, expectation.hard(), true)?
            {
                let regions = self.type_actual_regions(path.application()?.1, 0)?;
                self.applications.borrow_mut().push((key, args, regions));
            } else {
                self.push_application(path, application_namespace(value)?)?;
            }
            let application = ApplicationExpectation {
                outer: expectation,
                arguments: path.application()?.1,
            };
            let result = self.expr_with_expectation(value, Expectation::Application(&application));
            self.applications.borrow_mut().truncate(previous);
            return result;
        }
        if let Expr::Cast { value, target } = e {
            return self.cast_expression(value, target);
        }
        if matches!(
            e,
            Expr::TraitCall { .. } | Expr::MethodCall { .. } | Expr::AssociatedCall { .. }
        ) {
            return self.associated_expression(e, expectation.hard());
        }
        if let Some(id) = self.overloaded_index_borrow(e)? {
            return Ok(id);
        }
        if matches!(
            e,
            Expr::Binary { .. }
                | Expr::Unary { .. }
                | Expr::Deref { .. }
                | Expr::DynamicIndex { .. }
                | Expr::Index { .. }
        ) && let Some(id) = self.overloaded_operator(e)?
        {
            return Ok(id);
        }
        if matches!(e, Expr::Binary { .. }) {
            return self.builtin_binary_expression(e, expectation.hard());
        }
        self.expr_without_cast(e, expectation)
    }
    pub(super) fn validate_borrow_referent(
        &self,
        _kind: ir::BorrowKind,
        referent: &ir::Ty,
    ) -> Result<(), String> {
        if let ir::Ty::Slice { element } = referent {
            return ir::validate_shared_slice_element(
                element,
                self.facts(),
                &self.signatures.plan()?.budget,
            );
        }
        if referent.contains_slice() {
            return Err("unsupported: borrowing or reborrowing slice places".into());
        }
        self.validate_value_carriers(referent)
    }
    pub(super) fn borrow_expression(
        &mut self,
        kind: ir::BorrowKind,
        place: ir::Place,
    ) -> Result<ir::ExprId, String> {
        let referent = ir::projected_ty(&self.locals[place.local.0].ty, &place.projection)?.clone();
        if matches!(referent, ir::Ty::Dyn(_) | ir::Ty::Slice { .. }) {
            // Only a whole-object reborrow `&*d` / `&mut *d` of a local
            // carrier is admitted; it copies the vtable or length word with
            // the data (TRAIT-OBJECTS.md, MUTABLE-SLICES.md).
            if place.projection != [ir::Projection::Deref] {
                return Err(if matches!(referent, ir::Ty::Dyn(_)) {
                    "unsupported: borrowing trait object places"
                } else {
                    "unsupported: borrowing or reborrowing slice places"
                }
                .into());
            }
            if let ir::Ty::Slice { element } = &referent {
                ir::validate_shared_slice_element(
                    element,
                    self.facts(),
                    &self.signatures.plan()?.budget,
                )?;
            }
        } else {
            self.validate_borrow_referent(kind, &referent)?;
        }
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: ir::Ty::Ref {
                kind,
                referent: Arc::new(referent),
            },
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Borrow {
                kind,
                place,
                timing: ir::BorrowTiming::Immediate,
            },
        });
        Ok(id)
    }
    pub(super) fn call_argument(
        &mut self,
        source: &Expr,
        expected: &ir::Ty,
        reachable: bool,
    ) -> Result<(ir::ExprId, Option<ir::ImplicitBorrowKind>), String> {
        if let (
            Expr::Var(name),
            ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent,
            },
        ) = (source, expected)
        {
            let local = self.lookup(name)?;
            if let ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: actual,
            } = &self.locals[local.0].ty
            {
                let budget = &self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("call requires plan")?
                    .budget;
                if metered_type_equal(actual, referent, budget, 0)? {
                    let carrier = self.expr(source)?;
                    let id = self.borrow_expression(
                        ir::BorrowKind::Mut,
                        ir::Place {
                            local,
                            projection: vec![ir::Projection::Deref],
                        },
                    )?;
                    return Ok((
                        id,
                        Some(ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier }),
                    ));
                }
            }
        }
        self.expr_expected_when(source, reachable, Some(expected))
            .map(|id| (id, None))
    }
    pub(super) fn finish_checked_call(
        &mut self,
        target: ir::Callee,
        name: impl LocalInput,
        signature: ir::Signature,
        source_args: &[Expr],
        type_actual_regions: Arc<[ir::TypeActualRegion]>,
        receiver: Option<(ir::ExprId, Option<ir::ImplicitBorrowKind>)>,
    ) -> Result<ir::ExprId, String> {
        self.finish_checked_call_mode(
            target,
            name,
            signature,
            source_args,
            type_actual_regions,
            (receiver, false),
        )
    }

    /// Comparison autoref borrows the original argument occurrence directly.
    /// It must not synthesize a cloned AST whose nested calls lose authority.
    pub(super) fn finish_checked_call_mode(
        &mut self,
        target: ir::Callee,
        name: impl LocalInput,
        signature: ir::Signature,
        source_args: &[Expr],
        type_actual_regions: Arc<[ir::TypeActualRegion]>,
        (receiver, shared_arguments): (Option<(ir::ExprId, Option<ir::ImplicitBorrowKind>)>, bool),
    ) -> Result<ir::ExprId, String> {
        let name = name.diagnostic(self.names()?)?;
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("call evidence requires registry")?;
        // Resolve under the original caller facts before recording ordinary
        // actuals. Trait Self is not an ordinary source type argument, but a
        // selected recipe may have genuine ordinary arguments of its own.
        let (target, signature) = if let ir::Callee::TraitItem(call) = &target {
            let facts = self
                .facts()
                .validated()
                .ok_or("trait call evidence requires validated facts")?;
            let mut scratch = self.trait_scratch.borrow_mut();
            facts
                .validate_signature_with(
                    ir::SignatureSubject::Call(&target),
                    &signature,
                    &mut scratch,
                )
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            let resolved = facts
                .service()
                .resolve_trait_call_with(call, facts, &mut scratch)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            drop(scratch);
            let signature = resolved.signature();
            plan.budget.charge_work(
                signature
                    .params
                    .len()
                    .checked_add(resolved.callee().type_arguments().len())
                    .and_then(|work| work.checked_add(1))
                    .ok_or("resolved call evidence metadata work overflow")?,
            )?;
            self.charge_node(
                signature
                    .params
                    .len()
                    .checked_mul(std::mem::size_of::<ir::Ty>())
                    .and_then(|bytes| bytes.checked_add(std::mem::size_of::<ir::Signature>()))
                    .and_then(|bytes| bytes.checked_add(std::mem::size_of::<ir::Callee>()))
                    .ok_or("resolved call evidence metadata allocation overflow")?,
            )?;
            facts
                .validate_signature(ir::SignatureSubject::Call(resolved.callee()), signature)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            (resolved.callee().clone(), signature.clone())
        } else {
            (target, signature)
        };
        let (signature, nominal_realization) =
            match self.nominal_call_signature(&target, signature)? {
                nominal_calls::NominalCallSignature::Ready {
                    signature,
                    realization,
                } => (signature, realization),
                nominal_calls::NominalCallSignature::Blocked(blocked) => {
                    *self.nominal_blocked.borrow_mut() = Some(blocked);
                    return Err("missing graph input: canonical call signature realization".into());
                }
            };
        let actuals = target.type_arguments();
        if actuals.iter().any(ir::Ty::contains_slice) {
            return Err("unsupported: slice generic arguments".into());
        }
        plan.budget.charge_work(type_actual_regions.len())?;
        self.charge_node(
            type_actual_regions
                .len()
                .checked_mul(4 * std::mem::size_of::<ir::TypeActualRegion>() + 64)
                .ok_or("call region evidence allocation overflow")?,
        )?;
        let mut region_rows = type_actual_regions.to_vec();
        for (argument, ty) in actuals.iter().enumerate() {
            for location in ir::carrier_locations(ty, &plan.budget)?.iter() {
                if !matches!(location.channel, ir::CarrierChannel::Fixed(_)) {
                    continue;
                }
                plan.budget.charge_work(region_rows.len())?;
                if !region_rows
                    .iter()
                    .any(|row| row.argument == argument && row.location == *location)
                {
                    self.charge_node(4 * std::mem::size_of::<ir::TypeActualRegion>() + 64)?;
                    region_rows.push(ir::TypeActualRegion {
                        argument,
                        location: location.clone(),
                        source: ir::ActualRegion::Elided,
                    });
                }
            }
        }
        let type_actual_regions = region_rows.into();
        let offset = usize::from(receiver.is_some());
        if signature.params.len() != source_args.len() + offset {
            return Err(format!("wrong arity for {name:?}"));
        }
        self.infer_storage(
            source_args
                .len()
                .checked_mul(4)
                .and_then(|n| n.checked_add(8))
                .ok_or("call allocation overflow")?,
        )?;
        let mut arguments = Vec::with_capacity(signature.params.len());
        let mut pending = Vec::with_capacity(signature.params.len());
        self.charge_node(
            signature
                .params
                .len()
                .checked_mul(4 * std::mem::size_of::<ir::CallArgumentTrace>() + 64)
                .ok_or("call trace allocation overflow")?,
        )?;
        // This signature is the original source-stage signature. Inference may
        // not introduce new structural reference actuals, so a later argument
        // cannot retroactively make an opaque parameter a firm Ref expectation.
        let identity = match self.canonical {
            Some(cap) => cap.identity_arguments(source_args)?,
            None => false,
        };
        if identity && (receiver.is_some() || source_args.len() != 1) {
            return Err("shared replay identity argument shape changed".into());
        }
        let expectations = signature
            .params
            .iter()
            .map(|ty| match ty {
                ir::Ty::Ref { kind, .. } if !identity => {
                    ir::CallExpectation::FirmReference { kind: *kind }
                }
                _ => ir::CallExpectation::NoFirmReference,
            })
            .collect::<Vec<_>>();
        let mut completes = true;
        if let Some((id, reason)) = receiver {
            self.require(&self.flow(id), &signature.params[0])?;
            completes = self.flow(id).completes;
            arguments.push(id);
            if let Some(reason) = reason {
                pending.push((id, 0, reason));
            }
        }
        for (i, (source, ty)) in source_args
            .iter()
            .zip(&signature.params[offset..])
            .enumerate()
        {
            let (id, reason) = if shared_arguments {
                let ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    ..
                } = ty
                else {
                    return Err("comparison argument must have a shared-reference signature".into());
                };
                let id = if let Ok(place) = static_source_place(source)
                    && place_expression(source)
                {
                    let (place, _, _) = self.checked_place(&place)?;
                    self.borrow_expression(ir::BorrowKind::Shared, place)?
                } else {
                    // The selected impl fixes the operand type, so a
                    // constructor operand (`x == W { a: 1 }`) infers from it.
                    self.borrow_value(ir::BorrowKind::Shared, source, Some(ty))?
                };
                (id, None)
            } else if identity {
                (self.expr(source)?, None)
            } else {
                self.call_argument(source, ty, self.reachable && completes)?
            };
            let flow = self.flow(id);
            self.require(&flow, ty)?;
            completes &= flow.completes;
            arguments.push(id);
            if let Some(reason) = reason {
                pending.push((id, i + offset, reason));
            }
        }
        for ty in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.accessible(ty)?;
        }
        let id = ir::ExprId(self.exprs.len());
        for (borrow, argument, reason) in pending {
            let ir::ExprKind::Borrow { timing, .. } = &mut self.exprs[borrow.0].kind else {
                return Err("invalid implicit borrow argument".into());
            };
            *timing = ir::BorrowTiming::TwoPhase {
                activation: id,
                argument,
                reason,
            };
        }
        // A closure body owns its own expression arena and is never rebuilt by
        // the parent's canonical source transform; its mapped signature was
        // validated by `nominal_call_signature` and is re-proven by closure
        // call evidence at lowering. Every other body requires the receipt.
        if let Some(realization) = nominal_realization
            && !(self.canonical.is_none() && self.nominal_closure_body)
        {
            self.canonical
                .ok_or("nominal call realization requires canonical source publication")?
                .record_nominal_call_realization(
                    id,
                    &realization.target,
                    realization.original,
                    realization.mapped,
                    &mut self.trait_scratch.borrow_mut(),
                )?;
        }
        let traces = arguments
            .iter()
            .enumerate()
            .map(|(argument, value)| ir::CallArgumentTrace {
                argument,
                value: *value,
                expectation: expectations[argument],
            })
            .collect::<Vec<_>>();
        if let ir::Callee::Direct(direct) = &target {
            self.require_safe_callable(direct.declaration)?;
        }
        self.charge_node(4 * std::mem::size_of::<ir::CallEvidence>() + 64)?;
        self.call_evidence.push(ir::CallEvidence {
            call: id,
            arguments: traces.into(),
            type_actual_regions,
        });
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + name.len())?;
        self.exprs.push(ir::TypedExpr {
            ty: signature.result.clone(),
            completion: if completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind: ir::ExprKind::Call {
                target,
                name: name.to_owned(),
                args: arguments,
                signature,
            },
        });
        Ok(id)
    }
    pub(super) fn method_expression(
        &mut self,
        receiver: &Expr,
        name: &AstIdent,
        type_args: &[Type],
        args: &[Expr],
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        let ty = self
            .method_receiver_hint(receiver)?
            .ok_or("unsupported unresolved receiver type")?;
        let Some(selected) =
            self.method_selection_with_inference(&ty, name, type_args, args, expected)?
        else {
            return self.method_through_deref(receiver, &ty, name, type_args, args, expected);
        };
        let region_offset = match &selected.target {
            ir::Callee::Direct(target) => target.type_args.len().saturating_sub(type_args.len()),
            ir::Callee::TraitItem(_) | ir::Callee::Virtual(_) => 0,
        };
        if selected.depth == 1
            && matches!(selected.adjustment, ReceiverAdjustment::SharedAutoref)
            && matches!(
                selected.mode,
                ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Shared
                }
            )
            && array_len_receiver::explicit_mut_array_borrow(receiver)
            && matches!(&ty, ir::Ty::Ref { kind: ir::BorrowKind::Mut, referent }
                if matches!(referent.as_ref(), ir::Ty::Array { .. }))
        {
            let borrowed = self.shared_array_reference_temporary(receiver)?;
            return self.finish_checked_call(
                selected.target,
                name,
                selected.signature,
                args,
                self.type_actual_regions(type_args, region_offset)?,
                Some((borrowed, None)),
            );
        }
        // Value temporaries (rvalue receivers such as `Type::new().get()`) are
        // materialized in a block-scoped local and autoref'd. Values with drop
        // glue are excluded because the block would end their lifetime before
        // the enclosing statement does.
        if selected.depth == 0
            && matches!(
                selected.adjustment,
                ReceiverAdjustment::SharedAutoref | ReceiverAdjustment::MutAutoref
            )
            && !matches!(ty, ir::Ty::Ref { .. })
            && static_source_place(receiver).is_err()
            && self.trivially_dropped(&ty)?
        {
            let mutable = matches!(selected.adjustment, ReceiverAdjustment::MutAutoref);
            let value = self.expr(receiver)?;
            self.charge_node(std::mem::size_of::<ir::LocalDecl>() + 64)?;
            let local = ir::LocalId(self.locals.len());
            self.locals.push(ir::LocalDecl {
                name: "$primitive_receiver".into(),
                ty,
                mutable,
            });
            let borrowed = self.borrow_expression(
                if mutable {
                    ir::BorrowKind::Mut
                } else {
                    ir::BorrowKind::Shared
                },
                ir::Place {
                    local,
                    projection: Vec::new(),
                },
            )?;
            let call = self.finish_checked_call(
                selected.target,
                name,
                selected.original_signature.unwrap_or(selected.signature),
                args,
                self.type_actual_regions(type_args, region_offset)?,
                Some((borrowed, None)),
            )?;
            let result = self.exprs[call.0].ty.clone();
            let completion = self.exprs[call.0].completion;
            self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 128)?;
            let id = ir::ExprId(self.exprs.len());
            self.exprs.push(ir::TypedExpr {
                ty: result,
                completion,
                kind: ir::ExprKind::Block(ir::TypedBlock {
                    statements: vec![ir::TypedStmt::Let { local, value }],
                    tail: Some(call),
                }),
            });
            return Ok(id);
        }
        let adjusted = match selected.mode {
            ReceiverKind::None => return Err("method has no receiver".into()),
            ReceiverKind::Value { .. } => {
                let mut id = self.expr(receiver)?;
                for _ in 0..selected.depth {
                    let ir::Ty::Ref { referent, .. } = &self.exprs[id.0].ty else {
                        return Err("invalid receiver dereference".into());
                    };
                    let ty = referent.as_ref().clone();
                    let completion = self.exprs[id.0].completion;
                    self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                    let next = ir::ExprId(self.exprs.len());
                    self.exprs.push(ir::TypedExpr {
                        kind: ir::ExprKind::Deref { value: id },
                        ty,
                        completion,
                    });
                    id = next;
                }
                (id, None)
            }
            ReceiverKind::Borrowed { kind } => {
                if matches!(selected.target, ir::Callee::Virtual(_)) {
                    // Virtual dispatch keeps the complete object carrier.
                    // A shared method on `&mut dyn P` is selected after one
                    // dereference and shared autoref, and must create a real
                    // loan of the object instead of moving its mutable carrier.
                    let shared_reborrow = kind == ir::BorrowKind::Shared
                        && selected.depth == 1
                        && matches!(selected.adjustment, ReceiverAdjustment::SharedAutoref);
                    if !(matches!(selected.adjustment, ReceiverAdjustment::Identity)
                        && selected.depth == 0
                        || shared_reborrow)
                    {
                        return Err(
                            "unsupported: autoref or autoderef of a trait object receiver".into(),
                        );
                    }
                    if kind == ir::BorrowKind::Mut || shared_reborrow {
                        // Whole-object reborrow preserves both carrier words.
                        let Expr::Var(name) = receiver else {
                            return Err(
                                "unsupported: reborrowed trait object receiver that is not a local binding"
                                    .into(),
                            );
                        };
                        let local = self.lookup(name)?;
                        let id = self.borrow_expression(
                            kind,
                            ir::Place {
                                local,
                                projection: vec![ir::Projection::Deref],
                            },
                        )?;
                        (
                            id,
                            (kind == ir::BorrowKind::Mut)
                                .then_some(ir::ImplicitBorrowKind::MutableReceiver),
                        )
                    } else {
                        (self.expr(receiver)?, None)
                    }
                } else if matches!(selected.adjustment, ReceiverAdjustment::Identity)
                    && (kind == ir::BorrowKind::Shared
                        || (selected.depth == 0 && genuine_call(receiver)))
                {
                    let mut id = self.expr(receiver)?;
                    for _ in 0..selected.depth {
                        let ir::Ty::Ref { referent, .. } = &self.exprs[id.0].ty else {
                            return Err("invalid receiver dereference".into());
                        };
                        let ty = referent.as_ref().clone();
                        let completion = self.exprs[id.0].completion;
                        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                        let next = ir::ExprId(self.exprs.len());
                        self.exprs.push(ir::TypedExpr {
                            kind: ir::ExprKind::Deref { value: id },
                            ty,
                            completion,
                        });
                        id = next;
                    }
                    (id, None)
                } else {
                    if genuine_call(receiver) {
                        return Err(
                            "unsupported: autoref of a reference-return call temporary".into()
                        );
                    }
                    let source = static_source_place(receiver)?;
                    let (mut place, _, _) = self.checked_place(&source)?;
                    let derefs = selected.depth
                        + usize::from(matches!(selected.adjustment, ReceiverAdjustment::Identity));
                    self.charge_node(derefs * std::mem::size_of::<ir::Projection>() + 64)?;
                    place
                        .projection
                        .extend(std::iter::repeat_n(ir::Projection::Deref, derefs));
                    let id = self.borrow_expression(kind, place)?;
                    (
                        id,
                        if kind == ir::BorrowKind::Mut {
                            Some(ir::ImplicitBorrowKind::MutableReceiver)
                        } else {
                            None
                        },
                    )
                }
            }
        };
        self.finish_checked_call(
            selected.target,
            name,
            selected.original_signature.unwrap_or(selected.signature),
            args,
            self.type_actual_regions(type_args, region_offset)?,
            Some(adjusted),
        )
    }
    /// Method autoderef through one overloaded `Deref` step: the call is
    /// selected on the exact `Target` and receives the checked deref result.
    fn method_through_deref(
        &mut self,
        receiver: &Expr,
        ty: &ir::Ty,
        name: &AstIdent,
        type_args: &[Type],
        args: &[Expr],
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        const MISSING: &str = "no applicable method or unresolved method generic arguments";
        let owner = match ty {
            ir::Ty::Ref { referent, .. } => referent.as_ref(),
            ty => ty,
        };
        let Some(target) = self.deref_target_hint(owner)? else {
            return Err(MISSING.into());
        };
        let carrier = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(target.clone()),
        };
        let selected = self
            .method_selection_with_inference(&carrier, name, type_args, args, expected)?
            .ok_or(MISSING)?;
        let kind = match (selected.mode, selected.depth, selected.adjustment) {
            (
                ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Shared,
                },
                0,
                ReceiverAdjustment::Identity,
            ) => ir::BorrowKind::Shared,
            (
                ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Mut,
                },
                1,
                ReceiverAdjustment::MutAutoref,
            ) => ir::BorrowKind::Mut,
            _ => {
                return Err(
                    "unsupported: by-value or multi-step receiver through an overloaded Deref"
                        .into(),
                );
            }
        };
        let deref = self
            .receiver_overloaded_deref(receiver, ty, kind)?
            .ok_or(MISSING)?;
        let region_offset = match &selected.target {
            ir::Callee::Direct(target) => target.type_args.len().saturating_sub(type_args.len()),
            ir::Callee::TraitItem(_) | ir::Callee::Virtual(_) => 0,
        };
        self.finish_checked_call(
            selected.target,
            name,
            selected.original_signature.unwrap_or(selected.signature),
            args,
            self.type_actual_regions(type_args, region_offset)?,
            Some((
                deref,
                (kind == ir::BorrowKind::Mut).then_some(ir::ImplicitBorrowKind::MutableReceiver),
            )),
        )
    }
    pub(super) fn associated_expression(
        &mut self,
        expression: &Expr,
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        if let Expr::MethodCall {
            receiver,
            name,
            type_args,
            args,
        } = expression
        {
            return self.method_expression(receiver, name, type_args, args, expected);
        }
        let plan = self
            .signatures
            .plan
            .clone()
            .ok_or("associated call requires registry")?;
        let (owner, name, type_args, args, receiver) = match expression {
            Expr::MethodCall {
                receiver,
                name,
                type_args,
                args,
            } => {
                let ty = self
                    .method_receiver_hint(receiver)?
                    .ok_or("unsupported unresolved receiver type; use a typed local")?;
                let owner = match ty {
                    ir::Ty::Ref { referent, .. } => referent.as_ref().clone(),
                    ty => ty,
                };
                (owner, name, type_args, args, Some(receiver.as_ref()))
            }
            Expr::AssociatedCall {
                owner,
                name,
                type_args,
                args,
            } => (
                if inferred_owner::has_inferred_owner(owner) {
                    self.infer_associated_owner(owner, name, type_args.len(), args, expected)?
                } else {
                    checked_local_ty(owner, self.signatures)?
                },
                name,
                type_args,
                args,
                None,
            ),
            Expr::TraitCall {
                owner,
                name,
                type_args,
                args,
                ..
            } => (
                self.trait_call_owner(owner, args)?,
                name,
                type_args,
                args,
                None,
            ),
            _ => return Err("not an associated call".into()),
        };
        self.infer_storage(type_args.len() + args.len() + 1)?;
        let actuals = if matches!(expression, Expr::AssociatedCall { .. }) {
            self.resolved_method_actuals(&owner, name, type_args, args, expected)?
        } else if let Expr::TraitCall { trait_path, .. } = expression {
            let trait_ref = plan.resolve_trait_use(
                &owner,
                trait_path,
                self.signatures,
                self.facts(),
                &plan.budget,
            )?;
            self.inferred_trait_method_actuals(&trait_ref, name, type_args, args, expected, false)?
                .ok_or("E0282: type annotations needed for trait method arguments")?
        } else {
            type_args
                .iter()
                .map(|t| self.checked_type_actual(t))
                .collect::<Result<Vec<_>, _>>()?
        };
        let (target, signature, mode) = if let Expr::TraitCall { trait_path, .. } = expression {
            let trait_ref = plan.resolve_trait_use(
                &owner,
                trait_path,
                self.signatures,
                self.facts(),
                &plan.budget,
            )?;
            plan.select_trait(
                &trait_ref,
                name,
                &actuals,
                self.facts(),
                false,
                &plan.budget,
            )?
            .ok_or("E0277: trait obligation not proven")?
        } else {
            plan.select_associated(
                &owner,
                name,
                &actuals,
                self.facts(),
                &self.module,
                receiver.is_some(),
                &plan.budget,
            )?
        };
        let adjusted = if let Some(receiver) = receiver {
            match mode {
                ReceiverKind::None => return Err("associated function has no receiver".into()),
                ReceiverKind::Value { .. } => Some((self.expr(receiver)?, None)),
                ReceiverKind::Borrowed { kind } => {
                    if genuine_call(receiver) {
                        let id = self.expr(receiver)?;
                        Some((id, None))
                    } else {
                        let source = static_source_place(receiver)?;
                        let (mut place, ty, _) = self.checked_place(&source)?;
                        if matches!(ty, ir::Ty::Ref { .. }) {
                            if place.projection.contains(&ir::Projection::Deref) {
                                return Err(
                                    "multiple receiver autodereferences are unsupported".into()
                                );
                            }
                            place.projection.push(ir::Projection::Deref);
                        }
                        let id = self.borrow_expression(kind, place)?;
                        Some((
                            id,
                            if kind == ir::BorrowKind::Mut {
                                Some(ir::ImplicitBorrowKind::MutableReceiver)
                            } else {
                                None
                            },
                        ))
                    }
                }
            }
        } else {
            None
        };
        let region_offset = match &target {
            ir::Callee::Direct(target) => target.type_args.len().saturating_sub(type_args.len()),
            ir::Callee::TraitItem(_) | ir::Callee::Virtual(_) => 0,
        };
        let call = self.finish_checked_call(
            target,
            name,
            signature,
            args,
            self.type_actual_regions(type_args, region_offset)?,
            adjusted,
        )?;
        self.primitive_associated_actuals(expression, &owner, call)?;
        Ok(call)
    }
    /// `as` casts whose source or target is a raw pointer. Pairs are exactly
    /// those admitted by `ir::ptr_cast_valid`; everything else is E0606.
    fn pointer_cast_expression(
        &mut self,
        value: &Expr,
        target: ir::Ty,
    ) -> Result<ir::ExprId, String> {
        let value = match &target {
            ir::Ty::RawPtr { .. } if matches!(value, Expr::Integer(_)) => {
                self.expr_expected(value, Some(&ir::Ty::Int(ir::IntTy::Usize)))?
            }
            _ => self.expr(value)?,
        };
        let source = self.flow(value);
        if !ir::ptr_cast_valid(&source.ty, &target) {
            return Err(
                "E0606: unsupported pointer cast; supported: &T/&mut T to raw pointer, raw pointer to raw pointer, raw pointer to/from usize"
                    .into(),
            );
        }
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            kind: ir::ExprKind::PtrCast { value },
            ty: target,
            completion: if source.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
        });
        Ok(id)
    }
    pub(super) fn cast_expression(
        &mut self,
        value: &Expr,
        target: &Type,
    ) -> Result<ir::ExprId, String> {
        let target = checked_local_ty(target, self.signatures)?;
        if matches!(target, ir::Ty::RawPtr { .. })
            || matches!(self.scalar_hint(value)?, Some(ir::Ty::RawPtr { .. }))
        {
            return self.pointer_cast_expression(value, target);
        }
        // A char target must not infer an unsuffixed integer operand as u8.
        // Preserve its independently inferred source type before checking the pair.
        let value = match target {
            ir::Ty::Int(to) => self.expr_with_expectation(value, Expectation::CastableToInt(to))?,
            ir::Ty::Float(_) | ir::Ty::Char => self.expr(value)?,
            _ => return Err("unsupported cast target".into()),
        };
        let source = self.flow(value);
        let kind = match (&source.ty, &target) {
            (ir::Ty::Char, ir::Ty::Int(to)) => ir::ExprKind::CharToInt { value, to: *to },
            (ir::Ty::Int(ir::IntTy::U8), ir::Ty::Char) => ir::ExprKind::U8ToChar { value },
            (_, ir::Ty::Char) => {
                return Err("unsupported cast to char: source must be u8".into());
            }
            (ir::Ty::Char, _) => {
                return Err("unsupported cast from char: target must be an integer".into());
            }
            _ => {
                let from = ir::NumTy::of(&source.ty).ok_or("unsupported cast source")?;
                let numeric = ir::NumTy::of(&target).ok_or("unsupported cast target")?;
                match (from, numeric) {
                    (ir::NumTy::Int(_), ir::NumTy::Int(to)) => ir::ExprKind::IntCast { value, to },
                    _ => ir::ExprKind::FloatCast { value, to: numeric },
                }
            }
        };
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            kind,
            ty: target,
            completion: if source.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
        });
        Ok(id)
    }
    #[inline(never)]
    fn tuple_struct_value(
        &mut self,
        name: &AstPath,
        fields: &[Expr],
        expectation: Expectation<'_>,
    ) -> Result<(ir::ExprKind, Flow), String> {
        let expected = expectation.hard();
        self.infer_storage(fields.len())?;
        let evidence_fields = fields
            .iter()
            .enumerate()
            .map(|(index, value)| (InferField::Tuple(index), value))
            .collect::<Vec<_>>();
        let ir::Ty::Struct(ty) = self.constructor_type(name, None, &evidence_fields, expected)?
        else {
            return Err("expected tuple struct constructor".into());
        };
        if self.checked_struct_shape(&ty)? != StructShape::Tuple {
            return Err("tuple constructor requires a tuple struct".into());
        }
        let actual = ir::Ty::Struct(ty.clone());
        self.accessible(&actual)?;
        if !self.visible(&ty.def().visibility)? {
            return Err("private tuple struct constructor".into());
        }
        if fields.len() != ty.def().fields.len() {
            return Err("tuple struct constructor arity mismatch".into());
        }
        let mut values = Vec::with_capacity(fields.len());
        let mut completes = true;
        for (index, field) in fields.iter().enumerate() {
            let (_, field_ty) = self.field_type(&actual, &FieldSelector::Tuple(index))?;
            let formal =
                self.constructor_field_schema(ty.def().source_decl, None, index, expectation)?;
            let value = self.expr_with_expectation_when(
                field,
                self.reachable && completes,
                Expectation::SourceSchema(&field_ty, &formal),
            )?;
            let flow = self.flow(value);
            self.require(&flow, &field_ty)?;
            completes &= flow.completes;
            values.push((index, value));
        }
        Ok((
            ir::ExprKind::Struct { ty, fields: values },
            Flow {
                ty: actual,
                completes,
            },
        ))
    }
    #[inline(never)]
    /// Construct only checked storage, never synthetic source occurrences.
    pub(super) fn for_expr(
        &mut self,
        ty: ir::Ty,
        kind: ir::ExprKind,
    ) -> Result<ir::ExprId, String> {
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion: ir::Completion::Returns,
            kind,
        });
        Ok(id)
    }
    #[inline(never)]
    fn for_range_statement(
        &mut self,
        label: Option<AstIdent>,
        pattern: &Pattern,
        source: &ForSource,
        body: &Block,
        item_hint: Option<&ir::Ty>,
    ) -> Result<(ir::TypedStmt, bool), String> {
        if source
            .adapters
            .iter()
            .any(|a| !matches!(a, ForAdapter::Rev | ForAdapter::IntoIter))
        {
            return Err("for iterator adapters are not supported yet".into());
        }
        let reverse = source
            .adapters
            .iter()
            .filter(|a| matches!(a, ForAdapter::Rev))
            .count()
            % 2
            != 0;
        let ForIterable::Range {
            start,
            end,
            inclusive,
        } = &source.iterable
        else {
            if !source.adapters.is_empty() {
                return Err(
                    "for iterator adapters on non-range values are not supported yet".into(),
                );
            }
            let ForIterable::Value(value) = &source.iterable else {
                unreachable!()
            };
            if self.builtin_array_iter_receiver(value)?.is_some() {
                return self.for_array_statement(label, pattern, source, value, body, item_hint);
            }
            return self.for_array_statement(label, pattern, source, value, body, item_hint);
        };
        let loop_depth = self.loops.len();
        let scope_depth = self.scopes.len();
        let outer_reachable = self.reachable;
        let result = (|| {
            // Bounds are ordinary expressions evaluated before entering the loop.
            // They can target an enclosing loop but cannot see this loop's binder.
            let hint = item_hint
                .filter(|ty| matches!(ty, ir::Ty::Int(_)))
                .cloned()
                .or(self
                    .scalar_hint(start)?
                    .filter(|ty| matches!(ty, ir::Ty::Int(_))))
                .or(self
                    .scalar_hint(end)?
                    .filter(|ty| matches!(ty, ir::Ty::Int(_))));
            let start = self.expr_expected(start, hint.as_ref())?;
            let sf = self.flow(start);
            let end = self.expr_expected_when(
                end,
                outer_reachable && sf.completes,
                if sf.ty == ir::Ty::Never {
                    hint.as_ref()
                } else {
                    Some(&sf.ty)
                },
            )?;
            let ef = self.flow(end);
            let bound_ty = if sf.ty == ir::Ty::Never {
                &ef.ty
            } else {
                &sf.ty
            };
            self.require(&sf, bound_ty)?;
            self.require(&ef, bound_ty)?;
            let ir::Ty::Int(int_ty) = *bound_ty else {
                return Err("for range bounds must have an integer type".into());
            };
            self.charge_node(std::mem::size_of::<ir::TypedStmt>() * 10)?;
            let ty = ir::Ty::Int(int_ty);
            let cursor = self.temporary("$for_cursor", ty.clone(), true)?;
            let limit = self.temporary("$for_end", ty.clone(), false)?;
            let current = self.temporary("$for_current", ty.clone(), true)?;
            let done = self.temporary("$for_done", ir::Ty::Bool, true)?;
            let read_cursor = self.for_expr(ty.clone(), ir::ExprKind::Local(cursor))?;
            let read_limit = self.for_expr(ty.clone(), ir::ExprKind::Local(limit))?;
            let read_current = self.for_expr(ty.clone(), ir::ExprKind::Local(current))?;
            let read_done = self.for_expr(ir::Ty::Bool, ir::ExprKind::Local(done))?;
            let yes = self.for_expr(ir::Ty::Bool, ir::ExprKind::Bool(true))?;
            let no = self.for_expr(ir::Ty::Bool, ir::ExprKind::Bool(false))?;
            let one = self.for_expr(
                ty.clone(),
                ir::ExprKind::Int(ir::IntValue {
                    ty: int_ty,
                    bits: 1,
                }),
            )?;
            let increment_cursor = self.for_expr(ty.clone(), ir::ExprKind::Local(cursor))?;
            let compare_cursor = self.for_expr(ty.clone(), ir::ExprKind::Local(cursor))?;
            let compare_limit = self.for_expr(ty.clone(), ir::ExprKind::Local(limit))?;
            let snapshot_cursor = self.for_expr(ty.clone(), ir::ExprKind::Local(cursor))?;
            let initialize_cursor = self.for_expr(ty.clone(), ir::ExprKind::Local(cursor))?;
            let available_yes = self.for_expr(ir::Ty::Bool, ir::ExprKind::Bool(true))?;
            let exhausted_no = self.for_expr(ir::Ty::Bool, ir::ExprKind::Bool(false))?;
            let initial_no = self.for_expr(ir::Ty::Bool, ir::ExprKind::Bool(false))?;
            let next = self.for_expr(
                ty.clone(),
                ir::ExprKind::Binary {
                    op: if reverse {
                        BinaryOp::Sub
                    } else {
                        BinaryOp::Add
                    },
                    left: increment_cursor,
                    right: one,
                },
            )?;
            let at_end = self.for_expr(
                ir::Ty::Bool,
                ir::ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left: read_cursor,
                    right: read_limit,
                },
            )?;
            let in_range = self.for_expr(
                ir::Ty::Bool,
                ir::ExprKind::Binary {
                    op: if reverse && *inclusive {
                        BinaryOp::Ge
                    } else if reverse {
                        BinaryOp::Gt
                    } else if *inclusive {
                        BinaryOp::Le
                    } else {
                        BinaryOp::Lt
                    },
                    left: compare_cursor,
                    right: compare_limit,
                },
            )?;
            // Advance before the source body, so every continue (including a
            // labeled continue from a nested loop) takes the next item. Never
            // add one to the inclusive maximum: mark exhaustion instead.
            let advance = self.for_expr(
                ir::Ty::Unit,
                ir::ExprKind::If {
                    condition: at_end,
                    then_block: ir::TypedBlock {
                        statements: vec![ir::TypedStmt::Assign {
                            local: done,
                            value: yes,
                        }],
                        tail: None,
                    },
                    else_block: ir::TypedBlock {
                        statements: vec![ir::TypedStmt::Assign {
                            local: cursor,
                            value: next,
                        }],
                        tail: None,
                    },
                },
            )?;
            // A reversed half-open range holds the exclusive upper cursor.
            // Only subtract after the comparison, so empty ranges at MIN do
            // not overflow. Inclusive reverse snapshots before decrementing.
            let snapshot_cursor = if reverse && !*inclusive {
                let cursor_value = self.for_expr(ty.clone(), ir::ExprKind::Local(cursor))?;
                let one = self.for_expr(
                    ty.clone(),
                    ir::ExprKind::Int(ir::IntValue {
                        ty: int_ty,
                        bits: 1,
                    }),
                )?;
                self.for_expr(
                    ty.clone(),
                    ir::ExprKind::Binary {
                        op: BinaryOp::Sub,
                        left: cursor_value,
                        right: one,
                    },
                )?
            } else {
                snapshot_cursor
            };
            let available = self.for_expr(
                ir::Ty::Bool,
                ir::ExprKind::If {
                    condition: in_range,
                    then_block: ir::TypedBlock {
                        statements: vec![
                            ir::TypedStmt::Assign {
                                local: current,
                                value: snapshot_cursor,
                            },
                            ir::TypedStmt::Expr(advance),
                        ],
                        tail: Some(available_yes),
                    },
                    else_block: ir::TypedBlock {
                        statements: vec![],
                        tail: Some(no),
                    },
                },
            )?;
            let condition = self.for_expr(
                ir::Ty::Bool,
                ir::ExprKind::If {
                    condition: read_done,
                    then_block: ir::TypedBlock {
                        statements: vec![],
                        tail: Some(exhausted_no),
                    },
                    else_block: ir::TypedBlock {
                        statements: vec![],
                        tail: Some(available),
                    },
                },
            )?;
            let id = self.enter_loop(false);
            let key = self.label_key(label)?;
            let frame = self.loops.last_mut().unwrap();
            frame.label = key;
            frame.checking_condition = false;
            self.reachable = true;
            // One exhaustive arm gives the source pattern its genuine body-only
            // scope and rejects refutable patterns without fabricating binders.
            let (kind, flow) = self.lower_match_value(
                read_current,
                [BorrowedMatchArm {
                    pattern: Some(pattern),
                    guarded: false,
                    guard: None,
                    body: MatchBody::Block(body),
                }]
                .into_iter(),
                Some(&ir::Ty::Unit),
            )?;
            self.require(&flow, &ir::Ty::Unit)?;
            let matched = self.for_expr(flow.ty, kind)?;
            self.exprs[matched.0].completion = if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            };
            let mut initialization = if reverse {
                // Source bounds still evaluate exactly once, left to right.
                vec![
                    ir::TypedStmt::Let {
                        local: limit,
                        value: start,
                    },
                    ir::TypedStmt::Let {
                        local: cursor,
                        value: end,
                    },
                ]
            } else {
                vec![
                    ir::TypedStmt::Let {
                        local: cursor,
                        value: start,
                    },
                    ir::TypedStmt::Let {
                        local: limit,
                        value: end,
                    },
                ]
            };
            initialization.extend([
                ir::TypedStmt::Let {
                    local: current,
                    value: initialize_cursor,
                },
                ir::TypedStmt::Let {
                    local: done,
                    value: initial_no,
                },
                ir::TypedStmt::While {
                    id,
                    condition,
                    body: ir::TypedBlock {
                        statements: vec![ir::TypedStmt::Expr(matched)],
                        tail: None,
                    },
                },
            ]);
            let block = ir::TypedBlock {
                statements: initialization,
                tail: None,
            };
            let completes = sf.completes && ef.completes;
            let block = self.for_expr(ir::Ty::Unit, ir::ExprKind::Block(block))?;
            self.exprs[block.0].completion = if completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            };
            Ok((ir::TypedStmt::Expr(block), completes))
        })();
        self.loops.truncate(loop_depth);
        self.scopes.truncate(scope_depth);
        self.reachable = outer_reachable;
        result
    }
    #[inline(never)]
    fn for_array_statement(
        &mut self,
        label: Option<AstIdent>,
        pattern: &Pattern,
        for_source: &ForSource,
        source: &Expr,
        body: &Block,
        item_hint: Option<&ir::Ty>,
    ) -> Result<(ir::TypedStmt, bool), String> {
        let loop_depth = self.loops.len();
        let scope_depth = self.scopes.len();
        let outer_reachable = self.reachable;
        let result = (|| {
            // Only written array/repeat literals have a known shape here.
            // Other iterable forms retain their ordinary independent checking.
            let expected = item_hint.and_then(|item| {
                let len = match source {
                    Expr::Array(elements) => elements.len(),
                    Expr::Repeat { len, .. } => *len,
                    _ => return None,
                };
                Some(ir::Ty::Array {
                    element: Arc::new(item.clone()),
                    len,
                })
            });
            let value = match self.for_array_iter_value(for_source, source)? {
                Some(value) => value,
                None => self.expr_expected(source, expected.as_ref())?,
            };
            let flow = self.flow(value);
            let builtin = match &flow.ty {
                ir::Ty::Array { .. } => true,
                ir::Ty::Ref { kind, referent } => {
                    matches!(referent.as_ref(), ir::Ty::Array { .. })
                        || (*kind == ir::BorrowKind::Shared
                            && matches!(referent.as_ref(), ir::Ty::Slice { .. }))
                }
                _ => false,
            };
            if !builtin && self.signatures.plan()?.into_iterator_trait_id()?.is_some() {
                return self.for_iterator_statement(label, pattern, for_source, value, body);
            }
            let (element, len, borrowed) = match &flow.ty {
                ir::Ty::Array { element, len } => {
                    if !self.facts().must_copy(element)? {
                        return Err(
                            "unsupported: by-value for array iteration requires Copy elements"
                                .into(),
                        );
                    }
                    (element.as_ref().clone(), Some(*len), None)
                }
                ir::Ty::Ref { kind, referent } => {
                    match referent.as_ref() {
                        ir::Ty::Array { element, len } => (element.as_ref().clone(), Some(*len), Some(*kind)),
                        ir::Ty::Slice { element } if *kind == ir::BorrowKind::Shared => (element.as_ref().clone(), None, Some(*kind)),
                        _ => return Err("unsupported: for iteration requires an array or shared scalar/unit slice element-place authority".into()),
                    }
                }
                _ => return Err("for currently requires an integer range or an array".into()),
            };
            self.charge_node(std::mem::size_of::<ir::TypedStmt>() * 7)?;
            let array = self.temporary("$for_array", flow.ty.clone(), false)?;
            let index_ty = ir::Ty::Int(ir::IntTy::Usize);
            let index = self.temporary("$for_index", index_ty.clone(), true)?;
            let saved_index = self.temporary("$for_item_index", index_ty.clone(), true)?;
            // Every written checked expression occurrence owns distinct storage.
            let zero =
                self.for_expr(index_ty.clone(), ir::ExprKind::Int(ir::IntValue::usize(0)))?;
            let saved_zero =
                self.for_expr(index_ty.clone(), ir::ExprKind::Int(ir::IntValue::usize(0)))?;
            let length_kind = if let Some(len) = len {
                ir::ExprKind::Int(ir::IntValue::usize(len as u64))
            } else {
                let slice = self.for_expr(flow.ty.clone(), ir::ExprKind::Local(array))?;
                ir::ExprKind::SliceLen { slice }
            };
            let length = self.for_expr(index_ty.clone(), length_kind)?;
            if let Some(cap) = self.canonical {
                cap.record_for_len(for_source, length, value, &self.exprs)?;
            }
            let one = self.for_expr(index_ty.clone(), ir::ExprKind::Int(ir::IntValue::usize(1)))?;
            let compare_index = self.for_expr(index_ty.clone(), ir::ExprKind::Local(index))?;
            let snapshot_index = self.for_expr(index_ty.clone(), ir::ExprKind::Local(index))?;
            let increment_index = self.for_expr(index_ty.clone(), ir::ExprKind::Local(index))?;
            let in_range = self.for_expr(
                ir::Ty::Bool,
                ir::ExprKind::Binary {
                    op: BinaryOp::Lt,
                    left: compare_index,
                    right: length,
                },
            )?;
            let next = self.for_expr(
                index_ty.clone(),
                ir::ExprKind::Binary {
                    op: BinaryOp::Add,
                    left: increment_index,
                    right: one,
                },
            )?;
            let yes = self.for_expr(ir::Ty::Bool, ir::ExprKind::Bool(true))?;
            let no = self.for_expr(ir::Ty::Bool, ir::ExprKind::Bool(false))?;
            let condition = self.for_expr(
                ir::Ty::Bool,
                ir::ExprKind::If {
                    condition: in_range,
                    then_block: ir::TypedBlock {
                        statements: vec![
                            ir::TypedStmt::Assign {
                                local: saved_index,
                                value: snapshot_index,
                            },
                            ir::TypedStmt::Assign {
                                local: index,
                                value: next,
                            },
                        ],
                        tail: Some(yes),
                    },
                    else_block: ir::TypedBlock {
                        statements: vec![],
                        tail: Some(no),
                    },
                },
            )?;
            let id = self.enter_loop(false);
            let key = self.label_key(label)?;
            let frame = self.loops.last_mut().unwrap();
            frame.label = key;
            frame.checking_condition = false;
            self.reachable = true;
            let item = if let Some(kind) = borrowed {
                self.borrow_expression(
                    kind,
                    ir::Place {
                        local: array,
                        projection: vec![
                            ir::Projection::Deref,
                            ir::Projection::DynamicIndex(saved_index),
                        ],
                    },
                )?
            } else {
                let base = self.for_expr(flow.ty.clone(), ir::ExprKind::Local(array))?;
                let item_index = self.for_expr(index_ty, ir::ExprKind::Local(saved_index))?;
                self.for_expr(
                    element,
                    ir::ExprKind::DynamicIndex {
                        base,
                        index: item_index,
                    },
                )?
            };
            let (kind, body_flow) = self.lower_match_value(
                item,
                [BorrowedMatchArm {
                    pattern: Some(pattern),
                    guarded: false,
                    guard: None,
                    body: MatchBody::Block(body),
                }]
                .into_iter(),
                Some(&ir::Ty::Unit),
            )?;
            self.require(&body_flow, &ir::Ty::Unit)?;
            let matched = self.for_expr(body_flow.ty, kind)?;
            self.exprs[matched.0].completion = if body_flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            };
            let block = ir::TypedBlock {
                statements: vec![
                    ir::TypedStmt::Let {
                        local: array,
                        value,
                    },
                    ir::TypedStmt::Let {
                        local: index,
                        value: zero,
                    },
                    ir::TypedStmt::Let {
                        local: saved_index,
                        value: saved_zero,
                    },
                    ir::TypedStmt::While {
                        id,
                        condition,
                        body: ir::TypedBlock {
                            statements: vec![ir::TypedStmt::Expr(matched)],
                            tail: None,
                        },
                    },
                ],
                tail: None,
            };
            let block = self.for_expr(ir::Ty::Unit, ir::ExprKind::Block(block))?;
            self.exprs[block.0].completion = if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            };
            Ok((ir::TypedStmt::Expr(block), flow.completes))
        })();
        self.loops.truncate(loop_depth);
        self.scopes.truncate(scope_depth);
        self.reachable = outer_reachable;
        result
    }
    #[inline(never)]
    fn while_let_statement(
        &mut self,
        label: Option<AstIdent>,
        pattern: &Pattern,
        scrutinee: &Expr,
        body: &Block,
    ) -> Result<(ir::TypedStmt, bool), String> {
        let loop_depth = self.loops.len();
        let scope_depth = self.scopes.len();
        let outer_reachable = self.reachable;
        // Intrinsic loop completion must not inherit an outer dead path.
        self.reachable = true;
        let result = (|| {
            let id = self.enter_loop(false);
            let key = self.label_key(label)?;
            self.loops.last_mut().unwrap().label = key;
            // The genuine source scrutinee is checked under the same target
            // barrier as an ordinary while condition, before any binders.
            let scrutinee = self.expr(scrutinee)?;
            let condition_flow = self.flow(scrutinee);
            self.loops.last_mut().unwrap().checking_condition = false;
            let (kind, flow) = self.lower_match_value(
                scrutinee,
                [
                    BorrowedMatchArm {
                        pattern: Some(pattern),
                        guarded: false,
                        guard: None,
                        body: MatchBody::Block(body),
                    },
                    BorrowedMatchArm {
                        pattern: None,
                        guarded: false,
                        guard: None,
                        body: MatchBody::LoopExit(id),
                    },
                ]
                .into_iter(),
                Some(&ir::Ty::Unit),
            )?;
            self.require(&flow, &ir::Ty::Unit)?;
            let match_id = ir::ExprId(self.exprs.len());
            self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
            self.exprs.push(ir::TypedExpr {
                ty: flow.ty,
                completion: if flow.completes {
                    ir::Completion::Returns
                } else {
                    ir::Completion::Never
                },
                kind,
            });
            let frame = self
                .loops
                .last()
                .ok_or("contract: missing while-let frame")?;
            let can_exit = frame.can_exit;
            // This is checked IR storage, not a fabricated source block.
            self.charge_node(std::mem::size_of::<ir::TypedStmt>())?;
            let mut statements = Vec::new();
            let allocation_budget = self.names()?.budget().clone();
            statements.try_reserve_exact(1).map_err(|_| {
                super::super::source::SourceError::from_token(
                    super::super::token_session::TokenError::Budget(
                        allocation_budget.poison_allocation(),
                    ),
                )
                .to_string()
            })?;
            if statements.capacity() > 1 {
                self.charge_node(
                    (statements.capacity() - 1)
                        .checked_mul(std::mem::size_of::<ir::TypedStmt>())
                        .ok_or("while-let body capacity overflow")?,
                )?;
            }
            statements.push(ir::TypedStmt::Expr(match_id));
            let loop_id = ir::ExprId(self.exprs.len());
            self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
            self.exprs.push(ir::TypedExpr {
                ty: ir::Ty::Unit,
                completion: if can_exit {
                    ir::Completion::Returns
                } else {
                    ir::Completion::Never
                },
                kind: ir::ExprKind::Loop {
                    id,
                    body: ir::TypedBlock {
                        statements,
                        tail: None,
                    },
                },
            });
            // Runtime completion remains the genuine condition's completion.
            // The enclosing block separately retains this statement's source
            // Unit obligation rather than inventing runtime reachability.
            Ok((ir::TypedStmt::Expr(loop_id), condition_flow.completes))
        })();
        self.loops.truncate(loop_depth);
        self.scopes.truncate(scope_depth);
        self.reachable = outer_reachable;
        result
    }
    #[inline(never)]
    fn match_arm_body(
        &mut self,
        body: MatchBody<'_>,
        reachable: bool,
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        self.match_arm_body_with_expectation(body, reachable, expected.into())
    }
    fn match_arm_body_with_expectation(
        &mut self,
        body: MatchBody<'_>,
        reachable: bool,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        let (kind, flow) = match body {
            MatchBody::Expr(expr) => {
                return self.expr_with_expectation_when(expr, reachable, expectation);
            }
            MatchBody::Block(block) => {
                let (block, flow) =
                    self.block_with_expectation_when(block, reachable, expectation)?;
                (ir::ExprKind::Block(block), flow)
            }
            MatchBody::ImplicitUnit => (
                ir::ExprKind::Block(ir::TypedBlock {
                    statements: Vec::new(),
                    tail: None,
                }),
                Flow::value(ir::Ty::Unit),
            ),
            MatchBody::LoopExit(target) => {
                let frame = self
                    .loops
                    .iter_mut()
                    .rev()
                    .find(|frame| frame.id == target)
                    .ok_or("contract: missing while-let loop target")?;
                if frame.accepts_value || frame.checking_condition {
                    return Err("contract: invalid while-let exit target".into());
                }
                frame.can_exit |= reachable;
                (
                    ir::ExprKind::Break {
                        target,
                        value: None,
                    },
                    Flow::never(),
                )
            }
        };
        self.accessible(&flow.ty)?;
        self.admit_slice_value(&flow.ty)?;
        if matches!(flow.ty, ir::Ty::Array { .. } | ir::Ty::Tuple(_)) {
            self.validate_value_carriers(&flow.ty)?;
        }
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: flow.ty,
            completion: if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind,
        });
        Ok(id)
    }
    #[inline(never)]
    fn lower_match<'source>(
        &mut self,
        scrutinee: &Expr,
        arms: impl ExactSizeIterator<Item = BorrowedMatchArm<'source>> + Clone,
        expectation: Expectation<'_>,
    ) -> Result<(ir::ExprKind, Flow), String> {
        if arms.len() > 4096 {
            return Err("match arm count exceeds limit 4096".into());
        }
        let scrutinee = self.expr(scrutinee)?;
        self.lower_match_value_with_expectation(scrutinee, arms, expectation)
    }
    fn match_pattern_domain(&self, pattern: &Pattern, ty: &ir::Ty) -> Result<ir::Ty, String> {
        if *ty != ir::Ty::Never {
            return Ok(ty.clone());
        }
        Ok(match pattern {
            Pattern::TupleRest(_) => self.positional_pattern_type(pattern, ty)?,
            Pattern::Path(name)
            | Pattern::Variant { name, .. }
            | Pattern::VariantRest { name, .. } => {
                if let Some(tuple) = self.tuple_pattern_type(pattern, ty)? {
                    ir::Ty::Struct(tuple)
                } else {
                    ir::Ty::Enum(self.enum_variant(name)?.0)
                }
            }
            Pattern::Bool(_) => ir::Ty::Bool,
            Pattern::Int { literal, .. } => ir::Ty::Int(literal.suffix.unwrap_or(ir::IntTy::I32)),
            Pattern::Char(_) => ir::Ty::Char,
            Pattern::Range { lo, hi, .. } => range_pattern_hint(lo.as_ref().or(hi.as_ref())),
            Pattern::Tuple(fields) if fields.is_empty() => ir::Ty::Unit,
            Pattern::Struct { .. } | Pattern::Tuple(_) => self.pattern_ty(pattern, ty)?,
            _ => ty.clone(),
        })
    }
    #[inline(never)]
    fn let_else_statement(
        &mut self,
        pattern: &Pattern,
        annotation: &Type,
        source: &Expr,
        else_block: &Block,
    ) -> Result<(ir::TypedStmt, bool), String> {
        let annotation = if matches!(annotation, Type::Infer) {
            None
        } else {
            Some(checked_local_ty(annotation, self.signatures)?)
        };
        let value = self.expr_expected(source, annotation.as_ref())?;
        let flow = self.flow(value);
        if matches!(source, Expr::DynamicIndex { .. })
            && !matches!(pattern, Pattern::Wildcard)
            && !self.facts().must_copy(&flow.ty)?
        {
            return Err("non-Copy runtime indexed pattern extraction is unsupported; only direct let _ inspection is admitted".into());
        }
        let ty = self.match_pattern_domain(pattern, annotation.as_ref().unwrap_or(&flow.ty))?;
        self.accessible(&ty)?;
        self.require(&flow, &ty)?;
        let plan = self.collect_pattern(pattern, &ty)?;
        let mut matrix = super::pattern_usefulness::CoverageMatrix::new(
            ty.clone(),
            self.facts(),
            self.signatures.plan()?.budget.clone(),
            self.names()?.budget().clone(),
        )?;
        let total = if let Some(matrix) = matrix.as_mut() {
            plan.insert_coverage(matrix)?;
            matrix.exhaustive()?
        } else {
            plan.coverage().is_total(&ty)
        };
        let depth = self.scopes.len();
        let checked_else = self.match_arm_body(
            MatchBody::Block(else_block),
            self.reachable && flow.completes && !total,
            None,
        );
        self.scopes.truncate(depth);
        let else_branch = checked_else?;
        let else_flow = self.flow(else_branch);
        if else_flow.ty != ir::Ty::Never || else_flow.completes {
            return Err("let-else else block must have type Never and never complete".into());
        }
        let pattern = self.publish_pattern(plan)?;
        Ok((
            ir::TypedStmt::LetElse {
                value,
                ty,
                pattern,
                else_branch,
            },
            flow.completes,
        ))
    }
    #[inline(never)]
    fn lower_match_value<'source>(
        &mut self,
        scrutinee: ir::ExprId,
        arms: impl ExactSizeIterator<Item = BorrowedMatchArm<'source>> + Clone,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::ExprKind, Flow), String> {
        self.lower_match_value_with_expectation(scrutinee, arms, expected.into())
    }
    fn lower_match_value_with_expectation<'source>(
        &mut self,
        scrutinee: ir::ExprId,
        arms: impl ExactSizeIterator<Item = BorrowedMatchArm<'source>> + Clone,
        expectation: Expectation<'_>,
    ) -> Result<(ir::ExprKind, Flow), String> {
        // A cast may guide a literal operand, but its destination is not an
        // expected type for match arms. Infer their common source type first.
        let expectation = match expectation {
            Expectation::CastableToInt(_) => Expectation::None,
            other => other,
        };
        if arms.len() > 4096 {
            return Err("match arm count exceeds limit 4096".into());
        }
        let source = self.flow(scrutinee);
        // Or-patterns outside the binder-free scalar row domain are checked as
        // one arm per alternative sharing the source body (and guard).
        let budget = self.signatures.plan()?.budget.clone();
        let mut flattened: Vec<Option<Pattern>> = Vec::with_capacity(arms.len());
        for arm in arms.clone() {
            flattened.push(match arm.pattern {
                Some(pattern) if super::or_patterns::has_nested_alternatives(pattern, true) => {
                    Some(Pattern::Alternatives(super::or_patterns::flatten(
                        pattern, &budget,
                    )?))
                }
                _ => None,
            });
        }
        let mut shared: Vec<Option<shared_or_arms::SharedOrArm>> =
            (0..flattened.len()).map(|_| None).collect();
        // Every row of one source or-arm must bind the same names with the same
        // types and mutability, even when rows are checked separately and the
        // body never reads the binding (stock E0308).
        let mut row_bindings: Vec<Option<HashMap<NameKey, (ir::Ty, bool)>>> =
            (0..flattened.len()).map(|_| None).collect();
        let mut expanded_groups = Vec::new();
        let mut expanded = Vec::with_capacity(arms.len());
        for (group, (arm, flat)) in arms.zip(&flattened).enumerate() {
            let arm = BorrowedMatchArm {
                pattern: flat.as_ref().or(arm.pattern),
                ..arm
            };
            if let Some(Pattern::Alternatives(rows)) = arm.pattern
                && self.or_rows_need_expansion(rows, &source.ty, arm.guard.is_some())?
            {
                self.signatures.plan()?.budget.charge_work(rows.len())?;
                for row in rows {
                    expanded_groups.push(group);
                    expanded.push(BorrowedMatchArm {
                        pattern: Some(row),
                        ..arm
                    });
                }
            } else {
                expanded_groups.push(group);
                expanded.push(arm);
            }
        }
        if expanded.len() > 4096 {
            return Err("match arm count exceeds limit 4096".into());
        }
        let mut group_rows = vec![0usize; flattened.len()];
        for &group in &expanded_groups {
            group_rows[group] += 1;
        }
        let arms = expanded.iter().copied();
        let mut domain = source.ty.clone();
        // A divergent operand can coerce to the concrete type named by
        // a pattern without inventing a general inference variable.
        if domain == ir::Ty::Never {
            for arm in arms.clone() {
                let Some(source_pattern) = arm.pattern else {
                    continue;
                };
                let hint = match source_pattern {
                    Pattern::TupleRest(_) => {
                        Some(self.positional_pattern_type(source_pattern, &ir::Ty::Never)?)
                    }
                    Pattern::Path(name)
                    | Pattern::Variant { name, .. }
                    | Pattern::VariantRest { name, .. } => {
                        if let Some(tuple) =
                            self.tuple_pattern_type(source_pattern, &ir::Ty::Never)?
                        {
                            Some(ir::Ty::Struct(tuple))
                        } else {
                            Some(ir::Ty::Enum(self.enum_variant(name)?.0))
                        }
                    }
                    Pattern::Bool(_) => Some(ir::Ty::Bool),
                    Pattern::Int { literal, .. } => {
                        Some(ir::Ty::Int(literal.suffix.unwrap_or(ir::IntTy::I32)))
                    }
                    Pattern::Char(_) => Some(ir::Ty::Char),
                    Pattern::Range { lo, hi, .. } => {
                        Some(range_pattern_hint(lo.as_ref().or(hi.as_ref())))
                    }
                    Pattern::Tuple(fields) if fields.is_empty() => Some(ir::Ty::Unit),
                    Pattern::Struct { .. } | Pattern::Tuple(_) => {
                        Some(self.pattern_ty(source_pattern, &ir::Ty::Never)?)
                    }
                    _ => None,
                };
                if let Some(hint) = hint {
                    domain = hint;
                    break;
                }
            }
        }
        self.accessible(&domain)?;
        let mut matrix = super::pattern_usefulness::CoverageMatrix::new(
            domain.clone(),
            self.facts(),
            self.signatures.plan()?.budget.clone(),
            self.names()?.budget().clone(),
        )?;
        let mut catchall = false;
        let mut has_predicate = false;
        let mut seen_variants = super::super::token_session::BudgetVec::new(
            self.names()?.budget(),
            super::super::expansion_budget::Storage::Temporary,
        )
        .map_err(pattern_token_error)?;
        let mut covered = std::collections::HashSet::<u128>::new();
        let mut checked = Vec::with_capacity(arms.len());
        let mut result = ir::Ty::Never;
        let mut completes = false;
        let mut unsupported_alternative = false;
        for (arm_index, arm) in arms.clone().enumerate() {
            let group = expanded_groups[arm_index];
            let previous = shared[group].as_ref();
            let scope_depth = self.scopes.len();
            self.scopes.push(HashMap::new());
            let arm_result = (|| {
                let (pattern, coverage, exact_useful) = if let Some(Pattern::Alternatives(rows)) =
                    arm.pattern
                {
                    if arm.guarded {
                        return Err("unsupported: guards on or-patterns are unsupported".into());
                    }
                    ir::validate_alternative_domain(
                        &domain,
                        Some(&self.signatures.plan()?.budget),
                    )?;
                    if rows.len() < 2 {
                        return Err("invalid or-pattern alternative count".into());
                    }
                    let mut pending = super::super::token_session::BudgetVec::new(
                        self.names()?.budget(),
                        super::super::expansion_budget::Storage::Temporary,
                    )
                    .map_err(pattern_token_error)?;
                    let mut admitted = true;
                    for row in rows {
                        pending
                            .push(self.collect_pattern(row, &domain)?)
                            .map_err(pattern_token_error)?;
                        admitted &= self.alternative_leaf(row, 0)?;
                    }
                    if !admitted {
                        return Err(
                            "unsupported: or-patterns require binder-free scalar and tuple leaves"
                                .into(),
                        );
                    }
                    let matrix = matrix
                        .as_mut()
                        .ok_or("unsupported: or-pattern coverage domain")?;
                    let mut useful = false;
                    for row in pending.as_slice() {
                        let row_useful = row.insert_coverage(matrix)?;
                        unsupported_alternative |= !row_useful;
                        useful |= row_useful;
                    }
                    let mut published = self.retained_pattern_vec(rows.len())?;
                    while let Some(row) = pending.pop() {
                        published.push(self.publish_alternative_row(row)?);
                    }
                    self.signatures
                        .plan()?
                        .budget
                        .charge_work(published.len())?;
                    published.reverse();
                    (
                        ir::MatchPattern::Alternatives {
                            ty: domain.clone(),
                            rows: published,
                        },
                        PatternCoverage::Predicate { root_variant: None },
                        Some(useful),
                    )
                } else if let Some(source_pattern) = arm.pattern {
                    let plan = self.collect_pattern(source_pattern, &domain)?;
                    if arm.guarded {
                        // Validate real pending locals before publication, even on dead
                        // paths. An If cannot model guard borrows or binding immutability.
                        self.signatures
                            .plan()?
                            .budget
                            .charge_work(plan.bindings.rows.len())?;
                        for pending in plan.bindings.rows.as_slice() {
                            let binding = &pending.local;
                            if binding.mutable {
                                return Err("unsupported: guarded matches with mutable bindings are unsupported".into());
                            }
                            if !self.facts().must_copy(&binding.ty)? {
                                return Err("unsupported: guarded matches with non-Copy bindings are unsupported".into());
                            }
                        }
                    }
                    if arm.guard.is_some() {
                        for pending in plan.bindings.rows.as_slice() {
                            let binding = &pending.local;
                            if binding.borrow.is_none() && !self.facts().must_copy(&binding.ty)? {
                                return Err("unsupported: guarded match arms with by-value non-Copy bindings (use `ref`)".into());
                            }
                        }
                    }
                    let coverage = plan.coverage();
                    // A guarded arm covers nothing: the guard may fail.
                    let useful = if arm.guard.is_some() {
                        Some(true)
                    } else {
                        matrix
                            .as_mut()
                            .map(|matrix| plan.insert_coverage(matrix))
                            .transpose()?
                    };
                    (
                        self.publish_pattern_with_bindings(
                            plan,
                            previous.map(|row| &row.bindings),
                        )?,
                        coverage,
                        useful,
                    )
                } else {
                    let useful = matrix
                        .as_mut()
                        .map(|matrix| {
                            matrix.insert_row(0, |_| {
                                unreachable!("zero-test semantic fallback callback")
                            })
                        })
                        .transpose()?;
                    (
                        ir::MatchPattern::Wildcard,
                        PatternCoverage::CatchAll,
                        useful,
                    )
                };
                let fully_covered = match &domain {
                    ir::Ty::Bool => covered.len() == 2,
                    ir::Ty::Unit => covered.len() == 1,
                    ir::Ty::Enum(ty) => covered.len() == ty.def().variants.len(),
                    ir::Ty::Int(ty) => {
                        ty.bits() < 64 && covered.len() as u64 == (1u64 << ty.bits())
                    }
                    _ => false,
                };
                let key = match coverage {
                    PatternCoverage::Predicate { root_variant } => {
                        root_variant.map(|index| index as u128)
                    }
                    _ => coverage.full_key(),
                };
                let useful = exact_useful.unwrap_or_else(|| {
                    !catchall && !fully_covered && key.is_none_or(|key| !covered.contains(&key))
                });
                let (guard, body) = if let Some(previous) = previous {
                    (previous.guard, previous.body)
                } else {
                    let guard = arm
                        .guard
                        .map(|guard| -> Result<ir::ExprId, String> {
                            let id = self.expr_expected(guard, Some(&ir::Ty::Bool))?;
                            self.require(&self.flow(id), &ir::Ty::Bool)?;
                            Ok(id)
                        })
                        .transpose()?;
                    let body = self.match_arm_body_with_expectation(
                        arm.body,
                        self.reachable && source.completes && useful,
                        expectation,
                    )?;
                    (guard, body)
                };
                Ok::<_, String>((pattern, coverage, body, useful, guard))
            })();
            if arm.guard.is_some()
                && shared[group].is_none()
                && let Ok((_, _, body, _, guard)) = &arm_result
            {
                shared[group] = Some(shared_or_arms::SharedOrArm {
                    bindings: self.shared_pattern_bindings(scope_depth)?,
                    guard: *guard,
                    body: *body,
                });
            }
            if arm_result.is_ok() && group_rows[group] > 1 {
                let mut signature = HashMap::new();
                for (name, local) in self.shared_pattern_bindings(scope_depth)? {
                    let declaration = self
                        .locals
                        .get(local.0)
                        .ok_or("or-pattern binding outside checked locals")?;
                    signature.insert(name, (declaration.ty.clone(), declaration.mutable));
                }
                match &row_bindings[group] {
                    None => row_bindings[group] = Some(signature),
                    Some(first) => {
                        if first.len() != signature.len()
                            || first.keys().any(|name| !signature.contains_key(name))
                        {
                            self.scopes.truncate(scope_depth);
                            return Err(
                                "E0408: or-pattern alternatives bind different variables".into()
                            );
                        }
                        if first
                            .iter()
                            .any(|(name, binding)| signature[name] != *binding)
                        {
                            self.scopes.truncate(scope_depth);
                            return Err(
                                "E0308: or-pattern alternatives bind different types or mutability"
                                    .into(),
                            );
                        }
                    }
                }
            }
            self.scopes.truncate(scope_depth);
            let (pattern, coverage, body, useful, guard) = arm_result?;
            let arm_flow = self.flow(body);
            // Even unreachable arms impose their source type obligation.
            result = self.join(&result, &arm_flow.ty, "match arms")?;
            completes |= useful && arm_flow.completes;
            if matrix.is_none() && guard.is_none() {
                if let Some(key) = coverage.full_key() {
                    covered.insert(key);
                }
                match coverage {
                    PatternCoverage::CatchAll => catchall = true,
                    PatternCoverage::Variant(index) => {
                        seen_variants.push(index).map_err(pattern_token_error)?
                    }
                    PatternCoverage::Predicate { root_variant } => {
                        has_predicate = true;
                        if let Some(index) = root_variant {
                            seen_variants.push(index).map_err(pattern_token_error)?;
                        }
                    }
                    PatternCoverage::Scalar(_) => {}
                }
            }
            checked.push(ir::TypedMatchArm {
                pattern,
                guard,
                body,
            });
        }
        let checked = self.group_or_arms(checked, &expanded_groups, &domain)?;
        let exhaustive = if let Some(matrix) = matrix.as_ref() {
            matrix.exhaustive()?
        } else {
            catchall
                || match &domain {
                    ir::Ty::Never => true,
                    ir::Ty::Bool => covered.len() == 2,
                    ir::Ty::Unit => covered.len() == 1,
                    ir::Ty::Enum(ty) => covered.len() == ty.def().variants.len(),
                    ir::Ty::Int(ty) => {
                        ty.bits() < 64 && covered.len() as u64 == (1u64 << ty.bits())
                    }
                    _ => false,
                }
        };
        if !exhaustive {
            let all_variants_seen = if matrix.is_some() {
                false
            } else if let ir::Ty::Enum(ty) = &domain {
                let mut all = true;
                for index in 0..ty.def().variants.len() {
                    self.signatures
                        .plan()?
                        .budget
                        .charge_work(seen_variants.len() + 1)?;
                    if !seen_variants.as_slice().contains(&index) {
                        all = false;
                        break;
                    }
                }
                all
            } else {
                true
            };
            if matrix.is_none() && has_predicate && all_variants_seen {
                return Err(super::super::source::SourceError {
                    kind: super::super::source::SourceErrorKind::Unsupported,
                    primary: None,
                    message: "nested pattern exhaustiveness requires an irrefutable variant or catch-all fallback".into(),
                    _diagnostic_lease: None, location: None,
                }.to_string());
            }
            return Err(format!("non-exhaustive match on {domain:?}"));
        }
        if unsupported_alternative {
            return Err(
                "unsupported: unreachable or-pattern alternatives require warning diagnostics"
                    .into(),
            );
        }
        Ok((
            ir::ExprKind::Match {
                scrutinee,
                arms: checked,
            },
            Flow {
                ty: result,
                completes: source.completes && completes,
            },
        ))
    }
    pub(super) fn expr_without_cast(
        &mut self,
        e: &Expr,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        // A castable hint is not a hard result constraint or reference coercion site.
        let expected = expectation.hard();
        if let Some(ir::Ty::Ref { kind, referent }) = expectation.coercion()
            && let Expr::Var(name) = e
        {
            let local = self.lookup(name)?;
            if let ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: actual,
            } = &self.locals[local.0].ty
                && actual == referent
            {
                // Reborrow the original carrier before any value read can move it.
                return self.expr_expected(
                    &Expr::Borrow {
                        kind: *kind,
                        place: Place {
                            local: static_source_place(e)?.local,
                            fields: vec![FieldSelector::Deref],
                        },
                    },
                    expected,
                );
            }
        }
        let (kind, flow) = match e {
            Expr::Invoke { .. } => {
                return Err(
                    "unresolved callee-expression invocation requires canonical source resolution"
                        .into(),
                );
            }
            Expr::UnexpandedMacro(_) => return Err(UNEXPANDED_MACRO_CHECK_ERROR.into()),
            Expr::Try(operand) => self.lower_try(operand)?,
            Expr::Cast { .. } => return Err("cast bypassed expression dispatcher".into()),
            Expr::Borrow { kind, place } => {
                let (place, referent, _writable) = self.checked_place(place).map_err(|error|format!("B1 borrow requires an existing local-root static place; borrowed rvalues/non-local targets are unsupported: {error}"))?;
                self.validate_borrow_referent(*kind, &referent)?;
                (
                    ir::ExprKind::Borrow {
                        kind: *kind,
                        place,
                        timing: ir::BorrowTiming::Immediate,
                    },
                    Flow::value(ir::Ty::Ref {
                        kind: *kind,
                        referent: std::sync::Arc::new(referent),
                    }),
                )
            }
            Expr::BorrowValue { kind, value } => {
                return self.borrow_value(*kind, value, expected);
            }
            // An unsized `s[a..b]` value only exists behind a borrow or as the
            // base of a further index (handled at DynamicIndex/Index).
            Expr::RangeIndex { .. } => {
                return Err("unsupported: unborrowed range index (an unsized [T] value)".into());
            }
            Expr::Deref { value } => {
                let mut root = value.as_ref();
                let mut depth = 1;
                while let Expr::Deref { value } = root {
                    if depth >= 128 {
                        return Err("reference dereference depth exceeds limit128".into());
                    }
                    if let Some(plan) = &self.signatures.plan {
                        plan.budget.charge_work(1)?;
                    }
                    depth += 1;
                    root = value;
                }
                let value = self.expr(value)?;
                let mut carrier = value;
                for _ in 0..depth {
                    match self.exprs[carrier.0].kind {
                        ir::ExprKind::Deref { value } => carrier = value,
                        _ => break,
                    }
                }
                let slice_carrier =
                    matches!(self.exprs[carrier.0].kind, ir::ExprKind::SliceIndex { .. })
                        && matches!(
                            self.exprs[value.0].ty,
                            ir::Ty::Ref {
                                kind: ir::BorrowKind::Shared,
                                ..
                            }
                        );
                if !matches!(root, Expr::Var(_)) && !genuine_call(root) && !slice_carrier {
                    // Static component carriers use the same checked projection
                    // authority as field receivers. SliceIndex already yields
                    // a checked value carrying the selected element's origins,
                    // so it must not be reinterpreted as a static array place.
                    let source = static_source_place(e)?;
                    self.checked_place(&source)?;
                }
                let flow = self.flow(value);
                let referent = match &flow.ty {
                    ir::Ty::Ref { referent, .. } => referent,
                    ir::Ty::RawPtr { pointee, .. } => {
                        if self.unsafe_depth == 0 {
                            return Err("E0133: dereference of raw pointer is unsafe and requires unsafe block".into());
                        }
                        pointee
                    }
                    _ => return Err("dereference requires a reference".into()),
                };
                let ty = referent.as_ref().clone();
                self.accessible(&ty)?;
                (
                    ir::ExprKind::Deref { value },
                    Flow {
                        ty,
                        completes: flow.completes,
                    },
                )
            }
            Expr::Array(elements) => {
                if elements.len() > ir::MAX_ARRAY_LEN {
                    return Err("array literal exceeds supported length".into());
                }
                let hint = match expected {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                    _ => None,
                };
                if elements.is_empty() && hint.is_none() {
                    return Err("empty array literal inference is unsupported; provide a concrete expected array type".into());
                }
                let mut element = hint.cloned().unwrap_or(ir::Ty::Never);
                let mut values = Vec::new();
                let mut completes = true;
                for expression in elements {
                    let value = self.expr_with_expectation_when(
                        expression,
                        self.reachable && completes,
                        expectation.aggregate_child(hint, 0),
                    )?;
                    let flow = self.flow(value);
                    completes &= flow.completes;
                    if let Some(expected) = hint {
                        self.require(&flow, expected)?;
                    } else {
                        element = self.join(&element, &flow.ty, "array elements")?;
                    }
                    values.push(value);
                }
                (
                    ir::ExprKind::Array(values),
                    Flow {
                        ty: ir::Ty::Array {
                            element: std::sync::Arc::new(element),
                            len: elements.len(),
                        },
                        completes,
                    },
                )
            }
            Expr::Repeat { value, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err("array repeat exceeds supported length".into());
                }
                let hint = match expected {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                    _ => None,
                };
                let value =
                    self.expr_with_expectation(value, expectation.aggregate_child(hint, 0))?;
                let flow = self.flow(value);
                if let Some(expected) = hint {
                    self.require(&flow, expected)?;
                }
                let element = hint.cloned().unwrap_or(flow.ty);
                if *len > 1 && !self.facts().must_copy(&element)? {
                    return Err("array repeat of more than one element requires Copy".into());
                }
                (
                    ir::ExprKind::Repeat { value, len: *len },
                    Flow {
                        ty: ir::Ty::Array {
                            element: std::sync::Arc::new(element),
                            len: *len,
                        },
                        completes: flow.completes,
                    },
                )
            }
            Expr::DynamicIndex { base, index, .. }
                if matches!(base.as_ref(), Expr::RangeIndex { .. }) =>
            {
                let Expr::RangeIndex {
                    base,
                    start,
                    end,
                    inclusive,
                } = base.as_ref()
                else {
                    unreachable!()
                };
                let slice = self.subslice(base, start.as_deref(), end.as_deref(), *inclusive)?;
                return self.slice_index(slice, index);
            }
            Expr::DynamicIndex { base, index, .. } => {
                if Self::dynamic_place(base) && !Self::nested_array_place(base) {
                    return Err("nested/chained runtime indexed place bases are unsupported; snapshot the row first".into());
                }
                let base = match self.shared_slice_view(base)? {
                    Some(view) => view,
                    None => self.expr(base)?,
                };
                let base = self.array_reference_as_slice(base)?;
                if self.flow(base).ty.shared_slice_element().is_some() {
                    return self.slice_index(base, index);
                }
                if self.deref_source(base) {
                    return Err("runtime indexed access through references is unsupported in B1; snapshot a Copy referent first".into());
                }
                let source = self.flow(base);
                let ir::Ty::Array { element, .. } = &source.ty else {
                    // Stock auto-derefs `r[i]` for references to arrays; that
                    // legal form is outside this slice, not invalid.
                    if matches!(&source.ty, ir::Ty::Ref { referent, .. } if matches!(referent.as_ref(), ir::Ty::Array { .. }))
                    {
                        return Err("unsupported: runtime indexing through a reference to an array; copy the array into a local first".into());
                    }
                    return Err("runtime indexing requires an array".into());
                };
                let ty = element.as_ref().clone();
                self.accessible(&ty)?;
                let index = self.expr_expected_when(
                    index,
                    self.reachable && source.completes,
                    Some(&ir::Ty::Int(ir::IntTy::Usize)),
                )?;
                let index_flow = self.flow(index);
                self.require_index(&index_flow)?;
                (
                    ir::ExprKind::DynamicIndex { base, index },
                    Flow {
                        ty,
                        completes: source.completes && index_flow.completes,
                    },
                )
            }
            Expr::Index { base, index, .. } if matches!(base.as_ref(), Expr::RangeIndex { .. }) => {
                let Expr::RangeIndex {
                    base,
                    start,
                    end,
                    inclusive,
                } = base.as_ref()
                else {
                    unreachable!()
                };
                let slice = self.subslice(base, start.as_deref(), end.as_deref(), *inclusive)?;
                let constant = ir::ExprId(self.exprs.len());
                self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                self.exprs.push(ir::TypedExpr {
                    ty: ir::Ty::Int(ir::IntTy::Usize),
                    completion: ir::Completion::Returns,
                    kind: ir::ExprKind::Int(ir::IntValue::usize(*index as u64)),
                });
                return self.slice_index_checked(slice, constant);
            }
            Expr::Index { base, index, .. } => {
                if Self::dynamic_place(base) && !Self::nested_array_place(base) {
                    return Err("nested/chained runtime indexed place bases are unsupported; snapshot the row first".into());
                }
                let base = match self.shared_slice_view(base)? {
                    Some(view) => view,
                    None => self.expr(base)?,
                };
                let base = self.array_reference_as_slice(base)?;
                let flow = self.flow(base);
                if flow.ty.shared_slice_element().is_some() {
                    // Constant slice indexes use the same runtime-checked read.
                    let constant = ir::ExprId(self.exprs.len());
                    self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                    self.exprs.push(ir::TypedExpr {
                        ty: ir::Ty::Int(ir::IntTy::Usize),
                        completion: ir::Completion::Returns,
                        kind: ir::ExprKind::Int(ir::IntValue::usize(*index as u64)),
                    });
                    return self.slice_index_checked(base, constant);
                }
                let ty = self.index_type(&flow.ty, *index)?;
                (
                    ir::ExprKind::Index {
                        base,
                        index: *index,
                    },
                    Flow {
                        ty,
                        completes: flow.completes,
                    },
                )
            }
            Expr::EnumVariant {
                name,
                shape,
                fields,
            } => {
                let (owner, variant_name) = self.split_variant(name)?;
                self.infer_storage(fields.len())?;
                if fields.iter().any(|(selector, _)| {
                    matches!(
                        selector,
                        FieldSelector::Deref | FieldSelector::ConstantIndex(_)
                    )
                }) {
                    return Err("unsupported constructor inference selector".into());
                }
                let evidence_fields = fields
                    .iter()
                    .map(|(selector, value)| {
                        (
                            match selector {
                                FieldSelector::Named(name) => InferField::Named(name),
                                FieldSelector::Tuple(index) => InferField::Tuple(*index),
                                FieldSelector::Deref | FieldSelector::ConstantIndex(_) => {
                                    unreachable!("constructor fields are named or tuple selectors")
                                }
                            },
                            value,
                        )
                    })
                    .collect::<Vec<_>>();
                let ir::Ty::Enum(ty) =
                    self.constructor_type(&owner, Some(variant_name), &evidence_fields, expected)?
                else {
                    return Err("expected enum constructor".into());
                };
                let variant = self.variant_index(ty.def().source_decl, variant_name)?;
                self.accessible(&ir::Ty::Enum(ty.clone()))?;
                let definition = &ty.def().variants[variant];
                if *shape != definition.shape
                    && !(definition.shape == ir::VariantShape::Unit
                        && *shape == ir::VariantShape::Struct
                        && fields.is_empty())
                {
                    if *shape == ir::VariantShape::Unit
                        && definition.shape == ir::VariantShape::Tuple
                    {
                        return Err("first-class tuple variant constructors are unsupported".into());
                    }
                    return Err(format!("variant constructor {name:?} has wrong shape"));
                }
                let mut seen = std::collections::HashSet::new();
                let mut values = Vec::new();
                let mut completes = true;
                for (selector, value) in fields {
                    let index = match selector {
                        FieldSelector::Tuple(i) if definition.shape == ir::VariantShape::Tuple => {
                            *i
                        }
                        FieldSelector::Named(n) => {
                            self.member_index(ty.def().source_decl, Some(variant), *n)?
                        }
                        _ => return Err("invalid variant constructor selector".into()),
                    };
                    let field = definition
                        .fields
                        .get(index)
                        .ok_or("variant constructor arity mismatch")?;
                    if !seen.insert(index) {
                        return Err("duplicate variant constructor field".into());
                    }
                    self.accessible(&field.ty)?;
                    let formal = self.constructor_field_schema(
                        ty.def().source_decl,
                        Some(variant),
                        index,
                        expectation,
                    )?;
                    let value = self.expr_with_expectation_when(
                        value,
                        self.reachable && completes,
                        Expectation::SourceSchema(&field.ty, &formal),
                    )?;
                    let flow = self.flow(value);
                    self.require(&flow, &field.ty)?;
                    completes &= flow.completes;
                    values.push((index, value));
                }
                if seen.len() != definition.fields.len() {
                    return Err("missing variant constructor fields".into());
                }
                (
                    ir::ExprKind::EnumVariant {
                        ty: ty.clone(),
                        variant,
                        fields: values,
                    },
                    Flow {
                        ty: ir::Ty::Enum(ty),
                        completes,
                    },
                )
            }
            Expr::Match { scrutinee, arms } => self.lower_match(
                scrutinee,
                arms.iter().map(|arm| BorrowedMatchArm {
                    pattern: Some(&arm.pattern),
                    guarded: arm.guarded,
                    guard: arm.guard.as_deref(),
                    body: MatchBody::Expr(&arm.body),
                }),
                expectation,
            )?,
            Expr::IfLet {
                pattern,
                scrutinee,
                then_block,
                else_branch,
            } => self.lower_match(
                scrutinee,
                [
                    BorrowedMatchArm {
                        pattern: Some(pattern.as_ref()),
                        guarded: false,
                        guard: None,
                        body: MatchBody::Block(then_block),
                    },
                    BorrowedMatchArm {
                        pattern: None,
                        guarded: false,
                        guard: None,
                        body: else_branch
                            .as_deref()
                            .map_or(MatchBody::ImplicitUnit, MatchBody::Expr),
                    },
                ]
                .into_iter(),
                expectation,
            )?,
            Expr::Tuple(fields) => {
                if fields.is_empty() {
                    return Err("empty tuples must use ()".into());
                }
                let mut values = Vec::with_capacity(fields.len());
                let mut types = Vec::with_capacity(fields.len());
                let mut completes = true;
                for (index, field) in fields.iter().enumerate() {
                    let hint = match expected {
                        Some(ir::Ty::Tuple(types)) => types.get(index),
                        _ => None,
                    };
                    let value = self.expr_with_expectation_when(
                        field,
                        self.reachable && completes,
                        expectation.aggregate_child(hint, index),
                    )?;
                    let flow = self.flow(value);
                    completes &= flow.completes;
                    types.push(flow.ty);
                    values.push(value);
                }
                (
                    ir::ExprKind::Tuple(values),
                    Flow {
                        ty: ir::Ty::Tuple(types.into()),
                        completes,
                    },
                )
            }
            Expr::TraitCall { .. }
            | Expr::Apply { .. }
            | Expr::MethodCall { .. }
            | Expr::AssociatedCall { .. } => {
                return Err("application dispatcher invariant".into());
            }
            Expr::TupleStruct { name, fields } => {
                self.tuple_struct_value(name, fields, expectation)?
            }
            Expr::Struct { name, fields } => {
                self.infer_storage(fields.len())?;
                let evidence_fields = fields
                    .iter()
                    .map(|(name, value)| (InferField::Named(name), value))
                    .collect::<Vec<_>>();
                let ir::Ty::Struct(ty) =
                    self.constructor_type(name, None, &evidence_fields, expected)?
                else {
                    return Err("expected struct constructor".into());
                };
                if self.checked_struct_shape(&ty)? == StructShape::Tuple {
                    return Err("unsupported: braced tuple struct construction".into());
                }
                if !self.visible(&ty.def().visibility)? {
                    return Err(format!("private struct {name:?}"));
                }
                let mut seen = std::collections::HashSet::new();
                let mut values = Vec::with_capacity(fields.len());
                let mut completes = true;
                for (name, value) in fields {
                    let (index, expected) =
                        self.field_type(&ir::Ty::Struct(ty.clone()), &FieldSelector::Named(*name))?;
                    if !seen.insert(index) {
                        return Err(format!("duplicate field {name:?}"));
                    }
                    let formal = self.constructor_field_schema(
                        ty.def().source_decl,
                        None,
                        index,
                        expectation,
                    )?;
                    let value = self.expr_with_expectation_when(
                        value,
                        self.reachable && completes,
                        Expectation::SourceSchema(&expected, &formal),
                    )?;
                    let flow = self.flow(value);
                    self.require(&flow, &expected)?;
                    completes &= flow.completes;
                    values.push((index, value));
                }
                if seen.len() != ty.def().fields.len() {
                    return Err(format!("missing fields in struct {}", ty.def().name));
                }
                (
                    ir::ExprKind::Struct {
                        ty: ty.clone(),
                        fields: values,
                    },
                    Flow {
                        ty: ir::Ty::Struct(ty),
                        completes,
                    },
                )
            }
            Expr::Field { base, field } => {
                self.reject_noncopy_slice_field(e)?;
                let receiver = base.as_ref();
                let local_hint = match static_source_place(receiver) {
                    Ok(place) => self.checked_place(&place).ok().map(|(_, ty, _)| ty),
                    Err(_) => None,
                };
                if let Some(hint) =
                    local_hint.or_else(|| self.method_receiver_hint(receiver).ok().flatten())
                    && let ir::Ty::Struct(nominal) = match &hint {
                        ir::Ty::Ref { referent, .. } => referent.as_ref(),
                        ty => ty,
                    }
                    && !self.has_member(nominal.def().source_decl, field)?
                    && let Some(deref) =
                        self.receiver_overloaded_deref(receiver, &hint, ir::BorrowKind::Shared)?
                {
                    // `recv.f` through `Deref::Target` when `recv` has no `f`.
                    let ir::Ty::Ref { referent, .. } = self.exprs[deref.0].ty.clone() else {
                        return Err("overloaded deref must return a reference".into());
                    };
                    let completion = self.exprs[deref.0].completion;
                    let target = ir::ExprId(self.exprs.len());
                    self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                    self.exprs.push(ir::TypedExpr {
                        ty: referent.as_ref().clone(),
                        completion,
                        kind: ir::ExprKind::Deref { value: deref },
                    });
                    let (index, ty) = self.field_type(&referent, field)?;
                    let completes = completion == ir::Completion::Returns;
                    (
                        ir::ExprKind::Field {
                            base: target,
                            index,
                        },
                        Flow { ty, completes },
                    )
                } else {
                    let mut base = self.expr(base)?;
                    let mut flow = self.flow(base);
                    if let ir::Ty::Ref { referent, .. } = &flow.ty {
                        if !matches!(receiver, Expr::Var(_)) && !genuine_call(receiver) {
                            // A projected carrier has the same authority as any other
                            // static place. Validate its dereference without creating a
                            // local or borrow, then read the already evaluated base once.
                            let mut source = static_source_place(receiver)?;
                            source.fields.push(FieldSelector::Deref);
                            self.checked_place(&source)?;
                        }
                        let ty = referent.as_ref().clone();
                        self.accessible(&ty)?;
                        let deref = ir::ExprId(self.exprs.len());
                        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                        self.exprs.push(ir::TypedExpr {
                            ty: ty.clone(),
                            completion: self.exprs[base.0].completion,
                            kind: ir::ExprKind::Deref { value: base },
                        });
                        base = deref;
                        flow.ty = ty;
                    }
                    let (index, ty) = self.field_type(&flow.ty, field)?;
                    (
                        ir::ExprKind::Field { base, index },
                        Flow {
                            ty,
                            completes: flow.completes,
                        },
                    )
                }
            }
            Expr::Integer(literal) => {
                let value = expectation.literal(literal, false)?;
                (ir::ExprKind::Int(value), Flow::value(ir::Ty::Int(value.ty)))
            }
            Expr::Float(literal) => {
                let ty = match (literal.suffix, expected) {
                    (Some(ty), _) => ty,
                    (None, Some(ir::Ty::Float(ty))) => *ty,
                    _ => ir::FloatTy::F64,
                };
                let value = literal.value(ty, false)?;
                (ir::ExprKind::Float(value), Flow::value(ir::Ty::Float(ty)))
            }
            Expr::Bool(value) => (ir::ExprKind::Bool(*value), Flow::value(ir::Ty::Bool)),
            Expr::Char(value) => (
                ir::ExprKind::Char(ir::CharValue {
                    value: *value as u32,
                }),
                Flow::value(ir::Ty::Char),
            ),
            Expr::ByteStr(bytes) => {
                self.charge_node(
                    bytes
                        .len()
                        .checked_add(32)
                        .ok_or("byte string storage overflow")?,
                )?;
                (
                    ir::ExprKind::ByteStr(bytes.clone()),
                    Flow::value(ir::Ty::byte_string_ty(bytes.len())),
                )
            }
            Expr::Str(text) => {
                self.charge_node(
                    text.len()
                        .checked_add(32)
                        .ok_or("string literal storage overflow")?,
                )?;
                (
                    ir::ExprKind::Str(text.clone()),
                    Flow::value(ir::Ty::shared_str_ty()),
                )
            }
            Expr::ConstItem(_) | Expr::AssociatedConst(_) => {
                let canonical = self
                    .canonical
                    .ok_or("contract: constant occurrence requires sealed graph inputs")?;
                (
                    canonical.constant_kind(e)?,
                    Flow::value(canonical.constant_type(e)?),
                )
            }
            Expr::Var(n) if !is_local_shaped_path(n) && self.fn_item_value(n)?.is_some() => {
                let ty = self.fn_item_value(n)?.ok_or("function item vanished")?;
                (ir::ExprKind::FnItem, Flow::value(ty))
            }
            Expr::Var(n) => {
                let local = self.lookup(n)?;
                let ty = self.locals[local.0].ty.clone();
                (
                    ir::ExprKind::Local(local),
                    Flow {
                        completes: inhabited(&ty),
                        ty,
                    },
                )
            }
            Expr::Panic(panic) => {
                let names = self.names()?;
                validate_panic_site(panic, names)?;
                let site = panic_callsite(names.session(), panic.site).map_err(|error| {
                    super::super::source::SourceError::from_token(error).to_string()
                })?;
                let original = names.session().original(site, 512).map_err(|error| {
                    super::super::source::SourceError::from_token(error).to_string()
                })?;
                let (path, line, column) =
                    names
                        .session()
                        .sources()
                        .location(original)
                        .map_err(|error| {
                            super::super::source::SourceError::from_token(error).to_string()
                        })?;
                // Admit an upper bound before formatting a possibly non-UTF8 path.
                let bytes = path
                    .as_os_str()
                    .len()
                    .checked_mul(3)
                    .and_then(|bytes| bytes.checked_add(64))
                    .and_then(|bytes| bytes.checked_add(panic.message.len()))
                    .and_then(|bytes| bytes.checked_add(std::mem::size_of::<ir::PanicMessage>()))
                    .ok_or("panic diagnostic storage overflow")?;
                self.charge_node(bytes)?;
                // Preformatted: the diagnostic was already written to stderr.
                let location = if panic.preformatted {
                    String::new()
                } else {
                    format!("{}:{line}:{column}", path.display())
                };
                (
                    ir::ExprKind::Panic(Box::new(ir::PanicMessage {
                        message: panic.message.clone(),
                        location,
                    })),
                    Flow::never(),
                )
            }
            Expr::Output(output) => {
                validate_output_site(output, self.names()?)?;
                self.charge_node(
                    output
                        .output
                        .text
                        .len()
                        .checked_add(32)
                        .ok_or("output text storage overflow")?,
                )?;
                (
                    ir::ExprKind::Output(output.output.clone()),
                    Flow::value(ir::Ty::Unit),
                )
            }
            Expr::RawSlice { kind, arguments } => self.raw_slice(*kind, arguments)?,
            Expr::TypeAlignment(queried) => self.type_alignment(queried)?,
            Expr::NativeSyscall(arguments) => {
                if self.unsafe_depth == 0 {
                    return Err("E0133: native syscall requires unsafe block".into());
                }
                if arguments.len() != 7 {
                    return Err("native syscall requires number and six usize arguments".into());
                }
                self.charge_node(7 * std::mem::size_of::<ir::ExprId>())?;
                let ty = ir::Ty::Int(ir::IntTy::Usize);
                let mut values = Vec::new();
                let mut completes = true;
                for argument in arguments {
                    let id = self.expr_expected(argument, Some(&ty))?;
                    if self.exprs[id.0].ty != ty {
                        return Err("native syscall arguments must be usize".into());
                    }
                    completes &= self.flow(id).completes;
                    values.push(id);
                }
                (ir::ExprKind::NativeSyscall(values), Flow { ty, completes })
            }
            Expr::Println(text) => (
                ir::ExprKind::Println(text.clone()),
                Flow::value(ir::Ty::Unit),
            ),
            Expr::Format {
                pieces,
                arguments,
                modes,
                specs,
                stream,
                newline,
            } => self.check_format(pieces, arguments, modes, specs, *stream, *newline)?,
            Expr::FormatInto(source) => self.check_format_into(source)?,
            Expr::Unary { op, value, .. } => {
                if matches!(op, UnaryOp::Neg)
                    && let Expr::Integer(literal) = value.as_ref()
                {
                    let value = expectation.literal(literal, true)?;
                    let id = ir::ExprId(self.exprs.len());
                    self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                    self.exprs.push(ir::TypedExpr {
                        kind: ir::ExprKind::Int(value),
                        ty: ir::Ty::Int(value.ty),
                        completion: ir::Completion::Returns,
                    });
                    return Ok(id);
                }
                let mut hint = expected
                    .filter(|t| matches!(t, ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool))
                    .cloned();
                if hint.is_none() {
                    hint = self.scalar_hint(value)?;
                }
                let value = if matches!(expectation, Expectation::CastableToInt(_)) {
                    self.expr_with_expectation(value, expectation)?
                } else {
                    self.expr_expected(value, hint.as_ref())?
                };
                let f = self.flow(value);
                let t = match op {
                    UnaryOp::Neg => match &f.ty {
                        ir::Ty::Int(ty) if ty.is_signed() => f.ty.clone(),
                        ir::Ty::Float(_) => f.ty.clone(),
                        ir::Ty::Never => {
                            return Err("unary negation of never is unsupported".into());
                        }
                        _ => return Err("unary negation requires a signed integer".into()),
                    },
                    UnaryOp::Not if matches!(f.ty, ir::Ty::Int(_)) => f.ty.clone(),
                    UnaryOp::Not => ir::Ty::Bool,
                };
                self.require(&f, &t)?;
                if matches!(op, UnaryOp::Neg) && f.ty == ir::Ty::Never {
                    return Err("unary negation of never is unsupported".into());
                }
                let ty = if f.ty == ir::Ty::Never {
                    ir::Ty::Never
                } else {
                    t
                };
                (
                    ir::ExprKind::Unary { op: *op, value },
                    Flow {
                        ty,
                        completes: f.completes,
                    },
                )
            }
            Expr::Binary { .. } => {
                return self.builtin_binary_expression(e, expected);
            }
            Expr::Call { name, args } => {
                let key = self.path_key(name, Namespace::Value)?;
                let plan = self
                    .signatures
                    .plan
                    .as_ref()
                    .ok_or("call requires authoritative declaration registry")?;
                let declaration = *plan
                    .registry
                    .functions
                    .get(&self.path_key(name, Namespace::Value)?)
                    .ok_or_else(|| format!("unresolved function {name:?}"))?;
                let (type_args, actual_regions) = if self
                    .applications
                    .borrow()
                    .last()
                    .is_some_and(|(owner, _, _)| owner == &key)
                {
                    let (_, args, regions) = self.applications.borrow_mut().pop().unwrap();
                    (args, regions)
                } else if plan.param_env(declaration)?.arity() > 0 {
                    // A solved local reference hint is not an original expected
                    // context. Prefer argument-owned evidence so the canonical
                    // replay retains its original borrow frontier.
                    let from_arguments = if expectation.coercion().is_none()
                        && matches!(expected, Some(ir::Ty::Ref { .. }))
                    {
                        self.infer_call_args(declaration, args, None, false)?
                    } else {
                        None
                    };
                    let actuals = match from_arguments {
                        Some(actuals) => actuals,
                        None => self
                            .infer_call_args(declaration, args, expected, true)?
                            .ok_or("unsupported generic inference: insufficient evidence; explicit type arguments required")?,
                    };
                    (actuals, Arc::from([]))
                } else {
                    (Vec::new(), Arc::from([]))
                };
                let target = ir::CallTarget {
                    declaration,
                    type_args: type_args.into(),
                };
                let signature =
                    if plan.param_env(declaration)?.arity() == 0 && target.type_args.is_empty() {
                        self.signatures
                            .functions
                            .get(&self.path_key(name, Namespace::Value)?)
                            .cloned()
                            .ok_or("missing function signature")?
                    } else {
                        plan.instantiate_signature(&target, self.facts(), &plan.budget)?
                    };
                let facts = self
                    .signatures
                    .facts
                    .as_deref()
                    .ok_or("direct call requires validated facts")?;
                let proof = facts
                    .service()
                    .discharge_direct(&target, facts)
                    .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
                if !facts
                    .world()
                    .signatures_equal(proof.signature(), &signature)
                    .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
                {
                    return Err("canonical direct signature mismatch".into());
                }
                if self.signatures.param_env.is_some() {
                    for ty in signature
                        .params
                        .iter()
                        .chain(std::iter::once(&signature.result))
                    {
                        validate_root_carrier(ty, self.facts(), &plan.budget)?;
                    }
                }
                for ty in signature
                    .params
                    .iter()
                    .chain(std::iter::once(&signature.result))
                {
                    self.accessible(ty)?;
                }
                if signature.params.len() != args.len() {
                    return Err(format!(
                        "wrong arity for {name:?}: expected {}, got {}",
                        signature.params.len(),
                        args.len()
                    ));
                }
                return self.finish_checked_call(
                    target.into(),
                    name,
                    signature,
                    args,
                    actual_regions,
                    None,
                );
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.expr(condition)?;
                let c = self.flow(condition);
                self.require(&c, &ir::Ty::Bool)?;
                let reachable = self.reachable && c.completes;
                // `if c { Some(x) } else { None }`: without an outer
                // expectation, the branches' joint evidence types both.
                let inferred = if expected.is_none() {
                    self.branch_evidence_type(then_block, else_block)?
                } else {
                    None
                };
                let expected = expected.or(inferred.as_ref());
                let expected = if inferred.is_some() {
                    expected.map_or(Expectation::None, Expectation::Inferred)
                } else {
                    expectation.inherited(expected)
                };
                let (then_block, t) =
                    self.block_with_expectation_when(then_block, reachable, expected)?;
                let (else_block, e) =
                    self.block_with_expectation_when(else_block, reachable, expected)?;
                let ty = self.join(&t.ty, &e.ty, "if branches")?;
                let completes = c.completes && (t.completes || e.completes);
                (
                    ir::ExprKind::If {
                        condition,
                        then_block,
                        else_block,
                    },
                    Flow { ty, completes },
                )
            }
            Expr::Loop { label, body } => {
                let id = self.enter_loop(true);
                let key = self.label_key(*label)?;
                self.loops.last_mut().unwrap().label = key;
                self.loops.last_mut().unwrap().expected = expected.cloned();
                // Preserve whether the source grants a coercion site even when
                // the expectation is wrapped in an application or schema.
                self.loops.last_mut().unwrap().expected_is_inferred =
                    expectation.coercion().is_none();
                self.loops.last_mut().unwrap().expected_source_schema = match expectation {
                    Expectation::SourceSchema(_, formal) => Some(formal.clone()),
                    _ => None,
                };
                // A loop has its own intrinsic completion facts even when the
                // enclosing expression is unreachable. Only its own body paths
                // determine whether one of its breaks can complete.
                let (body, body_flow) = self.block_when(body, true)?;
                self.require(&body_flow, &ir::Ty::Unit)?;
                let frame = self.loops.pop().unwrap();
                let flow = Flow {
                    ty: frame.break_ty.unwrap_or(ir::Ty::Never),
                    completes: frame.can_exit,
                };
                (ir::ExprKind::Loop { id, body }, flow)
            }
            Expr::Break { label, value } => {
                let index = self.loop_target("break", *label)?;
                let target = self.loops[index].id;
                let expected = self.loops[index].expected.clone();
                let source_schema = self.loops[index].expected_source_schema.clone();
                let expectation = if let (Some(actual), Some(formal)) =
                    (expected.as_ref(), source_schema.as_ref())
                {
                    Expectation::SourceSchema(actual, formal)
                } else if self.loops[index].expected_is_inferred {
                    expected
                        .as_ref()
                        .map_or(Expectation::None, Expectation::Inferred)
                } else {
                    expected.as_ref().into()
                };
                let value = value
                    .as_ref()
                    .map(|e| self.expr_with_expectation(e, expectation))
                    .transpose()?;
                let flow = value.map_or(Flow::value(ir::Ty::Unit), |e| self.flow(e));
                let frame = &self.loops[index];
                let previous_break_ty = frame.break_ty.clone();
                if !frame.accepts_value && value.is_some() {
                    return Err("break with a value is only allowed in loop, not while".into());
                }
                // Check every lexical break and the operand before contributing.
                // A typed but unreachable tail is not the never type.
                if self.reachable && flow.completes {
                    self.loops[index].can_exit = true;
                }
                if flow.ty != ir::Ty::Never {
                    let previous = previous_break_ty.unwrap_or(ir::Ty::Never);
                    let ty = self.join(&previous, &flow.ty, "break values")?;
                    self.loops[index].break_ty = Some(ty);
                }
                (ir::ExprKind::Break { target, value }, Flow::never())
            }
            Expr::Continue { label } => {
                let index = self.loop_target("continue", *label)?;
                let target = self.loops[index].id;
                (ir::ExprKind::Continue { target }, Flow::never())
            }
            Expr::LabeledBlock { label, body } => {
                let id = self.enter_loop(true);
                let key = self.label_key(Some(*label))?;
                let frame = self.loops.last_mut().unwrap();
                frame.label = key;
                frame.is_block = true;
                frame.expected = expected.cloned();
                frame.expected_is_inferred = expectation.coercion().is_none();
                frame.expected_source_schema = match expectation {
                    Expectation::SourceSchema(_, formal) => Some(formal.clone()),
                    _ => None,
                };
                let (body, body_flow) =
                    self.block_with_expectation_when(body, true, expectation)?;
                let frame = self.loops.pop().unwrap();
                let ty = self.join(
                    &body_flow.ty,
                    &frame.break_ty.unwrap_or(ir::Ty::Never),
                    "labeled block breaks",
                )?;
                (
                    ir::ExprKind::LabeledBlock { id, body },
                    Flow {
                        ty,
                        completes: body_flow.completes || frame.can_exit,
                    },
                )
            }
            Expr::Block(b) => {
                let (block, flow) = self.block_with_expectation(b, expectation)?;
                (ir::ExprKind::Block(block), flow)
            }
            Expr::Unsafe(b) => {
                self.unsafe_depth += 1;
                let result = self.block_with_expectation(b, expectation);
                self.unsafe_depth -= 1;
                let (block, flow) = result?;
                (ir::ExprKind::Block(block), flow)
            }
        };
        self.finish_checked_expression(e, kind, flow)
    }

    fn finish_checked_expression(
        &mut self,
        e: &Expr,
        kind: ir::ExprKind,
        flow: Flow,
    ) -> Result<ir::ExprId, String> {
        self.accessible(&flow.ty)?;
        self.admit_slice_value(&flow.ty)?;
        if matches!(flow.ty, ir::Ty::Array { .. } | ir::Ty::Tuple(_)) {
            self.validate_value_carriers(&flow.ty)?;
        }
        let id = ir::ExprId(self.exprs.len());
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        self.exprs.push(ir::TypedExpr {
            ty: flow.ty,
            completion: if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind,
        });
        if matches!(e, Expr::ConstItem(_) | Expr::AssociatedConst(_)) {
            self.canonical
                .ok_or("contract: constant emission requires sealed graph inputs")?
                .record_constant(e, id)?;
        }
        Ok(id)
    }
}
/// Validate all declarations and publish one shared nominal type environment.
pub fn signature_table(program: &Program) -> Result<Signatures, String> {
    type_env(program)
}
pub fn check_signatures(program: &Program) -> Result<(), String> {
    signature_table(program).map(|_| ())
}
pub fn check_function(program: &Program, index: usize) -> Result<(), String> {
    let signatures = signature_table(program)?;
    let plan = signatures
        .plan
        .as_ref()
        .ok_or("source function check requires planning context")?;
    if !matches!(
        plan.callable_provider(plan.callable_decl(index)?)?,
        CallableProvider::Source { .. }
    ) {
        return Err("invalid source function index".into());
    }
    check_function_with_signatures(program, index, &signatures)
}
/// `signatures` must be the validated table for this same immutable program.
/// Direct semantic checking includes CFG initialization/move analysis once.
/// Graph callers use the pure type_function_with_env operation instead.
pub fn check_function_with_signatures(
    program: &Program,
    index: usize,
    signatures: &Signatures,
) -> Result<(), String> {
    let typed = type_function_with_signatures(program, index, signatures)?;
    let plan = signatures
        .plan
        .as_ref()
        .ok_or("semantic checking requires authoritative world")?;
    let declaration = plan.callable_decl(index)?;
    let publication = match plan.callable_provider(declaration)? {
        CallableProvider::Source { .. } => {
            match plan
                .publish_callable_body(declaration, &plan.budget)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
            {
                PublishedCallableBody::Source(publication) => Some(publication),
                PublishedCallableBody::Recipe(_) => {
                    return Err("Source checker received a recipe publication".into());
                }
            }
        }
        CallableProvider::Recipe { .. } => None,
        CallableProvider::UnavailableProvided { .. } => {
            return Err("unavailable provided body cannot be checked".into());
        }
    };
    if plan.callable_env(declaration)?.is_generic() || publication.is_none() {
        let template = if let Some(publication) = &publication {
            super::super::cfg::lower_source_template(publication, &plan.budget)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
        } else {
            let facts = plan.callable_facts(declaration)?;
            super::super::cfg::lower_template(Arc::new(typed), facts, &plan.budget)?
        };
        return super::super::ownership::verify_template_in_const_context(
            template,
            plan.callable_constness(declaration)? == Constness::Const,
        )
        .map(|_| ())
        .map_err(|analysis| {
            super::super::analysis_diagnostic(&plan.budget, "universal callable", index, &analysis)
        });
    }
    // Source ownership (including implicit Drop) requires its canonical replay
    // publication. Raw typed lowering omits that proof for call-free bodies.
    let publication = publication.ok_or("closed Source checker omitted publication")?;
    let facts = publication.facts().clone();
    let lowered = super::super::cfg::lower_source_closed(&publication, &plan.budget)
        .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
    let closed = super::super::cfg::close(Arc::new(lowered), facts, &plan.budget)?;
    let analysis = match super::super::ownership::analyze_closed_in_const_context(
        closed,
        plan.callable_constness(declaration)? == Constness::Const,
    ) {
        Ok(_) => return Ok(()),
        Err(analysis) => analysis,
    };
    if !analysis.passed() {
        return Err(super::super::analysis_diagnostic(
            &plan.budget,
            &typed.name,
            index,
            &analysis,
        ));
    }
    Ok(())
}
/// Check and lower one body using the validated signatures for this program.
/// The returned arena owns all resolved locals, expression types and call facts.
pub fn type_function_with_signatures(
    program: &Program,
    index: usize,
    signatures: &Signatures,
) -> Result<ir::CheckedFunction, String> {
    type_function_with_env(program, index, signatures)
}
pub fn type_callable_with_plan(
    program: &Program,
    declaration: ir::DeclId,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<ir::CheckedFunction, String> {
    type_callable_source(program, declaration, plan, budget, None)
}

struct SignatureSource<'a> {
    regions: &'a RegionDecls,
    type_params: &'a [AstIdent],
    bounds: &'a [TypeBound],
    params: &'a [(BindingName, Type)],
    result: &'a Type,
    constness: Constness,
}
impl<'a> From<&'a Function> for SignatureSource<'a> {
    fn from(function: &'a Function) -> Self {
        Self {
            regions: &function.regions,
            type_params: &function.type_params,
            bounds: &function.bounds,
            params: &function.params,
            result: &function.result,
            constness: function.constness,
        }
    }
}
impl<'a> From<&'a FunctionHead> for SignatureSource<'a> {
    fn from(function: &'a FunctionHead) -> Self {
        Self {
            regions: &function.regions,
            type_params: &function.type_params,
            bounds: &function.bounds,
            params: &function.params,
            result: &function.result,
            constness: function.constness,
        }
    }
}
fn signature_source_equal(
    plan: &PlanCtx,
    left: SignatureSource<'_>,
    right: SignatureSource<'_>,
) -> Result<bool, String> {
    let names = &*plan.program.names;
    // Replay validates every written equality occurrence before resolving or
    // comparing paths. In particular, an unequal prefix or unsupported earlier
    // child must not hide a forged handle in a later binding.
    for source in [&left, &right] {
        for bound in source.bounds {
            for path in &bound.traits {
                if let TraitPath::Source(path) = path {
                    for segment in &path.segments {
                        plan.budget.charge_work(
                            segment
                                .args
                                .len()
                                .checked_add(segment.associated_types.len())
                                .and_then(|work| work.checked_add(1))
                                .ok_or("signature binding metadata work overflow")?,
                        )?;
                        for argument in &segment.args {
                            argument.semantic_eq(argument, names)?;
                        }
                        for binding in &segment.associated_types {
                            names
                                .key(binding.name, LookupClass::Member)
                                .map_err(native_adapter::name_diagnostic)?;
                            binding.value.semantic_eq(&binding.value, names)?;
                        }
                    }
                }
            }
        }
    }
    let work = left
        .type_params
        .len()
        .checked_add(left.params.len())
        .and_then(|n| n.checked_add(left.bounds.len()))
        .and_then(|n| n.checked_add(left.regions.params.len()))
        .and_then(|n| n.checked_add(left.regions.outlives.len()))
        .and_then(|n| n.checked_add(1))
        .ok_or("signature comparison work overflow")?;
    plan.budget.charge_work(work)?;
    let mut equal = left.constness == right.constness
        && left.type_params.len() == right.type_params.len()
        && left.params.len() == right.params.len()
        && left.bounds.len() == right.bounds.len()
        && left.regions.params.len() == right.regions.params.len()
        && left.regions.outlives.len() == right.regions.outlives.len();
    for (a, b) in left.type_params.iter().zip(right.type_params) {
        equal &= name_scope::ident_eq(names, *a, *b, LookupClass::TypeParameter)?;
    }
    for ((a, at), (b, bt)) in left.params.iter().zip(right.params) {
        equal &= a.checked_key(names)? == b.checked_key(names)?;
        equal &= at.semantic_eq(bt, names)?;
    }
    equal &= left.result.semantic_eq(right.result, names)?;
    for (a, b) in left.regions.params.iter().zip(&right.regions.params) {
        equal &= name_scope::ident_eq(names, *a, *b, LookupClass::LifetimeParameter)?;
    }
    equal &= match (left.regions.receiver, right.regions.receiver) {
        (Some(a), Some(b)) => name_scope::region_eq(names, a, b)?,
        (None, None) => true,
        _ => false,
    };
    for ((al, as_), (bl, bs)) in left.regions.outlives.iter().zip(&right.regions.outlives) {
        equal &= name_scope::region_eq(names, *al, *bl)?;
        equal &= name_scope::region_eq(names, *as_, *bs)?;
    }
    for (a, b) in left.bounds.iter().zip(right.bounds) {
        plan.budget.charge_work(a.traits.len().saturating_add(1))?;
        equal &= a.subject.semantic_eq(&b.subject, names)?;
        equal &= a.traits.len() == b.traits.len();
        for (a, b) in a.traits.iter().zip(&b.traits) {
            equal &= plan.resolve_trait_path(a)? == plan.resolve_trait_path(b)?;
            match (a, b) {
                (TraitPath::Source(a), TraitPath::Source(b)) => {
                    equal &= a.segments.len() == b.segments.len();
                    for (a, b) in a.segments.iter().zip(&b.segments) {
                        plan.budget.charge_work(a.associated_types.len())?;
                        equal &= a.associated_types.len() == b.associated_types.len();
                        for (a, b) in a.associated_types.iter().zip(&b.associated_types) {
                            equal &=
                                name_scope::ident_eq(names, a.name, b.name, LookupClass::Member)?;
                            equal &= a.value.semantic_eq(&b.value, names)?;
                        }
                    }
                    let a = a.segments.iter().flat_map(|segment| &segment.args);
                    let b = b.segments.iter().flat_map(|segment| &segment.args);
                    equal &= a.clone().count() == b.clone().count();
                    for (a, b) in a.zip(b) {
                        equal &= a.semantic_eq(b, names)?;
                    }
                }
                (TraitPath::Builtin(a), TraitPath::Builtin(b)) => equal &= a == b,
                _ => equal = false,
            }
        }
    }
    Ok(equal)
}

pub(super) fn type_callable_source(
    program: &Program,
    declaration: ir::DeclId,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    canonical: Option<&CanonicalSourceCapability>,
) -> Result<ir::CheckedFunction, String> {
    type_callable_source_and_closures(program, declaration, plan, budget, canonical)
        .map(|(body, _)| body)
}
/// Prepare one canonical source callable before executable local obligations.
/// The retained signature result is metadata, not a placeholder or admission;
/// source recipes, rather than runtime locals, seed this emission-free solve.
#[cfg(test)]
pub(in crate::compiler) fn prepare_callable_nominals(
    plan: &PlanCtx,
    declaration: ir::DeclId,
    receiver: Option<&NominalShape>,
    inputs: &NominalPreparationInputs<'_>,
) -> Result<NominalPreparation, String> {
    let signatures = plan.environment(Some(declaration))?;
    let function = plan.callable_body(&plan.program, declaration)?;
    let signature = plan.function_schema(declaration)?.signature;
    let mut checker = Checker::new(&signatures, signature.result, function.module);
    checker.install_lexical_scope(function.lexical_scope)?;
    checker.declaration = Some(declaration);
    checker.unsafe_depth = usize::from(function.unsafety);
    local_literals::nominal_manifest::prepare_callable(&checker, declaration, receiver, inputs)
}

/// Finalize only when every explicit graph input is present. Missing inputs
/// remain Blocked, with no eager nominal hinting fallback or partial body.
#[cfg(test)]
pub(in crate::compiler) fn finalized_callable_nominal_hints(
    plan: &PlanCtx,
    declaration: ir::DeclId,
    receiver: Option<&NominalShape>,
    inputs: &NominalPreparationInputs<'_>,
) -> Result<NominalPreparedHints, String> {
    let signatures = plan.environment(Some(declaration))?;
    let function = plan.callable_body(&plan.program, declaration)?;
    let signature = plan.function_schema(declaration)?.signature;
    let mut checker = Checker::new(&signatures, signature.result, function.module);
    checker.install_lexical_scope(function.lexical_scope)?;
    checker.declaration = Some(declaration);
    checker.unsafe_depth = usize::from(function.unsafety);
    local_literals::nominal_manifest::finalize_callable(&checker, declaration, receiver, inputs)
}

pub(in crate::compiler) enum NominalCallableOutput {
    Ready(
        Box<ir::CheckedFunction>,
        Vec<closures::check::CheckedClosure>,
        Vec<ir::OpaqueHiddenType>,
    ),
    Blocked(NominalPreparation),
}

pub(super) fn type_callable_source_and_closures(
    program: &Program,
    declaration: ir::DeclId,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    canonical: Option<&CanonicalSourceCapability>,
) -> Result<(ir::CheckedFunction, Vec<closures::check::CheckedClosure>), String> {
    type_callable_source_outputs(program, declaration, plan, budget, canonical)
        .map(|(body, closures, _)| (body, closures))
}
pub(super) fn type_callable_source_outputs(
    program: &Program,
    declaration: ir::DeclId,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    canonical: Option<&CanonicalSourceCapability>,
) -> Result<
    (
        ir::CheckedFunction,
        Vec<closures::check::CheckedClosure>,
        Vec<ir::OpaqueHiddenType>,
    ),
    String,
> {
    match type_callable_source_and_closures_inner(
        program,
        declaration,
        plan,
        budget,
        canonical,
        None,
    )? {
        NominalCallableOutput::Ready(body, closures, hidden) => Ok((*body, closures, hidden)),
        NominalCallableOutput::Blocked(_) => {
            Err("internal: ordinary callable checking suspended without nominal inputs".into())
        }
    }
}

/// Consume the same borrowed graph-read inputs as preparation.
pub(in crate::compiler) fn type_callable_source_with_nominal_inputs(
    program: &Program,
    declaration: ir::DeclId,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    canonical: Option<&CanonicalSourceCapability>,
    receiver: Option<&NominalShape>,
    inputs: &NominalPreparationInputs<'_>,
) -> Result<NominalCallableOutput, String> {
    type_callable_source_and_closures_inner(
        program,
        declaration,
        plan,
        budget,
        canonical,
        Some((receiver, inputs)),
    )
}

fn type_callable_source_and_closures_inner(
    program: &Program,
    declaration: ir::DeclId,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    canonical: Option<&CanonicalSourceCapability>,
    nominal: Option<(Option<&NominalShape>, &NominalPreparationInputs<'_>)>,
) -> Result<NominalCallableOutput, String> {
    plan.check_budget(budget)?;
    budget.charge_work(1)?;
    if !Arc::ptr_eq(&program.names, &plan.program.names)
        || !Arc::ptr_eq(&program.modules, &plan.program.modules)
        || match (&program.scopes, &plan.program.scopes) {
            (None, None) => false,
            (Some(actual), Some(original)) => !Arc::ptr_eq(actual, original),
            _ => true,
        }
    {
        return Err(super::super::source::SourceError::from_token(
            super::super::token_session::TokenError::ForeignHandle,
        )
        .to_string());
    }
    if matches!(
        plan.callable_provider(declaration)?,
        CallableProvider::Recipe { .. }
    ) {
        if nominal.is_some() {
            return Err(
                "unsupported: source nominal inputs require a source callable provider".into(),
            );
        }
        let env = plan.callable_env(declaration)?;
        budget.retain_bytes(env.total_arity() * std::mem::size_of::<ir::Ty>() + 128)?;
        let target = ir::CallTarget {
            declaration,
            type_args: env.ordered_params().map(ir::Ty::Param).collect(),
        };
        let facts = plan.callable_facts(declaration)?;
        return plan
            .checked_body(&target, &facts, budget)
            .map(|body| NominalCallableOutput::Ready(Box::new(body), Vec::new(), Vec::new()));
    }
    let f = plan.callable_body(program, declaration)?;
    plan.validate_callable_associated_bounds(program, declaration, budget)?;
    let containing_module =
        match plan.registry.callable_sources[plan.callable_index(declaration)?] {
            CallableSource::Free(_) => None,
            CallableSource::Associated { .. } | CallableSource::TraitDefault { .. } => {
                Some(plan.callable_context(declaration)?.module)
            }
        };
    name_validation::validate_function_names(program, f, containing_module)?;
    // Semantic equality cannot establish occurrence authority: another bound
    // in this same source owner can have the same member spelling and RHS.
    for bound in &f.bounds {
        for path in &bound.traits {
            plan.validate_source_bound_path(declaration, path, budget)?;
        }
    }
    let original = plan.callable_body(&plan.program, declaration)?;
    let same_name = name_scope::path_key(&*program.names, &f.name, Namespace::Value)?
        == name_scope::path_key(&*program.names, &original.name, Namespace::Value)?;
    if !same_name
        || f.module != original.module
        || f.lexical_scope != original.lexical_scope
        || f.visibility != original.visibility
        || !signature_source_equal(plan, f.into(), original.into())?
    {
        return Err(
            "source callable signature differs from its immutable planning declaration".into(),
        );
    }
    if let CallableSource::TraitDefault { trait_index, item } =
        plan.registry.callable_sources[plan.callable_index(declaration)?]
    {
        let supplied = &program.traits[trait_index].items[item].signature;
        let original = &plan.program.traits[trait_index].items[item].signature;
        name_validation::validate_function_head_names(program, supplied)?;
        for bound in &supplied.bounds {
            for path in &bound.traits {
                plan.validate_source_bound_path(declaration, path, budget)?;
            }
        }
        if !name_scope::ident_eq(
            &*program.names,
            supplied.name,
            original.name,
            LookupClass::Member,
        )? || !signature_source_equal(plan, supplied.into(), original.into())?
            || supplied.constness != f.constness
        {
            return Err(
                "source trait signature constness differs from its immutable planning declaration"
                    .into(),
            );
        }
    }
    validate_const_forwarding(&plan.registry, f, budget)?;
    let mut signatures = plan.environment(Some(declaration))?;
    if let Some(cap) = canonical {
        signatures.facts = Some(cap.facts.clone());
    }
    let mut signature = plan.function_schema(declaration)?.signature;
    let context = plan.callable_context(declaration)?;
    walk_depth_checked(DepthNode::Block(&f.body), None, Some(&program.names))?;
    let module = context.module;
    let mut c = Checker::new(&signatures, signature.result.clone(), module);
    c.install_lexical_scope(f.lexical_scope)?;
    c.declaration = Some(declaration);
    c.canonical = canonical;
    c.closure_source_recipe = canonical.map(|cap| cap.inventory.owner.clone());
    c.unsafe_depth = usize::from(f.unsafety);
    let root_hints = if let Some((receiver, inputs)) = nominal {
        let mut hints = match local_literals::nominal_manifest::finalize_callable(
            &c,
            declaration,
            receiver,
            inputs,
        )? {
            NominalPreparedHints::Ready(hints) => hints,
            NominalPreparedHints::Blocked(blocked) => {
                return Ok(NominalCallableOutput::Blocked(blocked));
            }
        };
        let realized = hints
            .source_signature
            .take()
            .ok_or("invalid metadata: finalized source hints lack callable signature")?;
        if realized.parameters.len() != signature.params.len() {
            return Err("invalid metadata: realized callable parameter count changed".into());
        }
        // Preserve original source contracts/evidence. The ordinary proofs
        // below must accept every realized value through the graph capability.
        signature.params = realized.parameters;
        signature.result = realized.result;
        c.result = signature.result.clone();
        c.nominal_inputs = Some(inputs);
        Some(hints)
    } else {
        None
    };
    c.prove_nominal_bounds(&signature.result)?;
    let mut params = Vec::new();
    let offset = if context.receiver != ReceiverKind::None {
        let local = c.bind(LocalName::Receiver, signature.params[0].clone())?;
        c.locals[local.0].mutable =
            matches!(context.receiver, ReceiverKind::Value { mutable: true });
        params.push(local);
        1
    } else {
        0
    };
    if signature.params.len() != f.params.len() + offset {
        return Err("callable parameter count mismatch".into());
    }
    if !f.param_mut.is_empty() && f.param_mut.len() != f.params.len() {
        return Err("parameter mutability table does not match parameters".into());
    }
    for (parameter, ((name, _), ty)) in f.params.iter().zip(&signature.params[offset..]).enumerate()
    {
        let local = c.bind(name, ty.clone())?;
        c.locals[local.0].mutable = f.param_mut.get(parameter).copied().unwrap_or(false);
        if let Some(cap) = canonical {
            sr::work(budget, 1).map_err(|e| cap.render(e))?;
            let index = parameter;
            if !cap
                .inventory
                .bindings
                .get(index)
                .is_some_and(|site| site.parameter == Some(parameter))
            {
                return Err(cap.render(FactsError::invalid(
                    budget,
                    "source parameter prefix occurrence changed",
                )));
            }
            sr::push(&mut cap.bindings.borrow_mut(), (index, local), budget)
                .map_err(|e| cap.render(e))?;
        }
        params.push(local);
    }
    let typed = match root_hints {
        Some(hints) => c.block_with_resolved_hints(&f.body, Some(&signature.result).into(), hints),
        None => c.block_expected(&f.body, Some(&signature.result)),
    };
    let (body, flow) = match typed {
        Ok(typed) => typed,
        Err(error) => {
            if let Some(blocked) = c.nominal_blocked.get_mut().take() {
                return Ok(NominalCallableOutput::Blocked(blocked));
            }
            return Err(format!("in callable {:?}: {error}", f.name));
        }
    };
    c.require(&flow, &c.result)?;
    validate_shift_counts(&c.exprs, &c.count_queries)?;
    c.finalize_opaque_returns()?;
    let checked = ir::CheckedFunction {
        runtime_sites: ir::RuntimeSites::from_source(c.runtime_sites)?,
        name: f
            .name
            .display(&*program.names)
            .map_err(native_adapter::name_diagnostic)?
            .to_string(),
        ref_contract: signature.ref_contract.clone(),
        params,
        result: c.result,
        locals: c.locals,
        exprs: c.exprs,
        call_evidence: c.call_evidence.into(),
        body_substitution: None,
        body,
    };
    validate_const_typed_calls(&plan.registry, f, &checked, budget)?;
    let closures = c.checked_closures;
    let hidden = c.opaque_hidden;
    if canonical.is_some() {
        return Ok(NominalCallableOutput::Ready(
            Box::new(checked),
            closures,
            hidden,
        ));
    }
    let facts = plan.callable_facts(declaration)?;
    ir::validate_call_evidence(&checked, &facts, budget)?;
    let mut cx = plan.clone();
    ir::canonicalize_checked_calls(&checked, &facts, &mut cx, budget)
        .map(|body| NominalCallableOutput::Ready(Box::new(body), closures, hidden))
        .map_err(|e| e.into_legacy_diagnostic(budget))
}
pub fn type_function_with_plan(
    program: &Program,
    index: usize,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<ir::CheckedFunction, String> {
    plan.check_budget(budget)?;
    budget.charge_work(1)?;
    let decl = plan.function_decl(index)?;
    type_callable_with_plan(program, decl, plan, budget)
}
/// Check one body with the same nominal environment used for all signatures.
pub fn type_function_with_env(
    program: &Program,
    index: usize,
    signatures: &TypeEnv,
) -> Result<ir::CheckedFunction, String> {
    if let Some(plan) = &signatures.plan {
        return type_callable_with_plan(program, plan.callable_decl(index)?, plan, &plan.budget);
    }
    let f = program
        .functions
        .get(index)
        .ok_or_else(|| format!("invalid function index {index}"))?;
    if !f.type_params.is_empty() && signatures.param_env.is_none() {
        let plan = signatures
            .plan
            .as_ref()
            .ok_or("generic body requires authoritative planning context")?;
        return type_function_with_plan(program, index, plan, &plan.budget);
    }
    validate_depth(DepthNode::Block(&f.body))?;
    let signature = signatures
        .functions
        .get(&name_scope::path_key(
            &*program.names,
            &f.name,
            Namespace::Value,
        )?)
        .ok_or_else(|| format!("missing signature for {:?}", f.name))?;
    if signature.params.len() != f.params.len() {
        return Err(format!(
            "signature parameter count mismatch for {:?}",
            f.name
        ));
    }
    let mut c = Checker::new(signatures, signature.result.clone(), f.module);
    c.prove_nominal_bounds(&signature.result)?;
    c.install_lexical_scope(f.lexical_scope)?;
    c.unsafe_depth = usize::from(f.unsafety);
    let mut params = Vec::with_capacity(f.params.len());
    for ((n, _), t) in f.params.iter().zip(&signature.params) {
        params.push(
            c.bind(n, t.clone())
                .map_err(|e| format!("in function {:?}: {e}", f.name))?,
        );
    }
    let (body, flow) = c
        .block_expected(&f.body, Some(&signature.result))
        .map_err(|e| format!("in function {:?}: {e}", f.name))?;
    c.require(&flow, &c.result)
        .map_err(|e| format!("in function {:?}: {e}", f.name))?;
    validate_shift_counts(&c.exprs, &c.count_queries)
        .map_err(|e| format!("in function {:?}: {e}", f.name))?;
    Ok(ir::CheckedFunction {
        runtime_sites: ir::RuntimeSites::from_source(c.runtime_sites)?,
        name: f
            .name
            .display(&*program.names)
            .map_err(native_adapter::name_diagnostic)?
            .to_string(),
        ref_contract: signature.ref_contract.clone(),
        params,
        result: c.result,
        locals: c.locals,
        exprs: c.exprs,
        call_evidence: c.call_evidence.into(),
        body_substitution: None,
        body,
    })
}
pub fn check(program: &Program) -> Result<(), String> {
    let signatures = signature_table(program)?;
    let count = signatures
        .plan
        .as_ref()
        .map_or(program.functions.len(), PlanCtx::callable_count);
    for i in 0..count {
        check_function_with_signatures(program, i, &signatures)?;
    }
    Ok(())
}

/// Default type of a range pattern from its first endpoint, before checking.
fn range_pattern_hint(bound: Option<&super::RangeBound>) -> ir::Ty {
    match bound {
        Some(super::RangeBound::Char(_)) => ir::Ty::Char,
        Some(super::RangeBound::IntLimit { ty, .. }) => ir::Ty::Int(*ty),
        Some(super::RangeBound::Int { literal, .. }) => {
            ir::Ty::Int(literal.suffix.unwrap_or(ir::IntTy::I32))
        }
        None => ir::Ty::Int(ir::IntTy::I32),
    }
}

/// Resolve a source range pattern against its exact scalar domain into one
/// inclusive test. Half-open upper bounds subtract one; an empty range is E0579.
pub(super) fn range_pattern_test(
    lo: Option<&super::RangeBound>,
    hi: Option<&super::RangeBound>,
    inclusive: bool,
    ty: &ir::Ty,
) -> Result<ir::PatternTestKind, String> {
    use super::RangeBound;
    if inclusive && hi.is_none() {
        return Err("inclusive range pattern requires an upper bound".into());
    }
    match ty {
        ir::Ty::Char => {
            let scalar = |bound: &RangeBound| match bound {
                RangeBound::Char(value) => Ok(u32::from(*value)),
                RangeBound::Int { .. } | RangeBound::IntLimit { .. } => {
                    Err("mismatched types: integer range bound in a character pattern".to_string())
                }
            };
            let low = lo.map(scalar).transpose()?.unwrap_or(0);
            let high = match hi {
                Some(bound) => {
                    let value = scalar(bound)?;
                    if inclusive {
                        value
                    } else {
                        // Step back over the surrogate gap so the bound stays a scalar.
                        match value {
                            0 => return Err("lower range bound must be less than upper".into()),
                            0xe000 => 0xd7ff,
                            other => other - 1,
                        }
                    }
                }
                None => 0x10ffff,
            };
            if low > high {
                return Err("lower range bound must be less than or equal to upper".into());
            }
            Ok(ir::PatternTestKind::CharRange {
                lo: ir::CharValue { value: low },
                hi: ir::CharValue { value: high },
            })
        }
        ir::Ty::Int(int) => {
            let value = |bound: &RangeBound| match bound {
                RangeBound::Int { literal, negative } => {
                    resolve_integer(literal, *negative, Some(ty))
                }
                RangeBound::IntLimit { ty: limit, max } => {
                    if limit != int {
                        return Err("mismatched types: integer range bound type".to_string());
                    }
                    let min = if int.is_signed() {
                        1u128 << (int.bits() - 1)
                    } else {
                        0
                    };
                    Ok(ir::IntValue {
                        ty: *int,
                        bits: if *max {
                            min.wrapping_sub(1) & int.mask()
                        } else {
                            min
                        },
                    })
                }
                RangeBound::Char(_) => {
                    Err("mismatched types: character range bound in an integer pattern".to_string())
                }
            };
            let min = if int.is_signed() {
                1u128 << (int.bits() - 1)
            } else {
                0
            };
            let low = match lo {
                Some(bound) => value(bound)?,
                None => ir::IntValue {
                    ty: *int,
                    bits: min,
                },
            };
            let high = match hi {
                Some(bound) => {
                    let high = value(bound)?;
                    if inclusive {
                        high
                    } else {
                        if high.ordinal() == 0 || high.ordinal() <= low.ordinal() {
                            return Err("lower range bound must be less than upper".into());
                        }
                        ir::IntValue {
                            ty: *int,
                            bits: high.bits.wrapping_sub(1) & int.mask(),
                        }
                    }
                }
                None => ir::IntValue {
                    ty: *int,
                    bits: min.wrapping_sub(1) & int.mask(),
                },
            };
            if low.ordinal() > high.ordinal() {
                return Err("lower range bound must be less than or equal to upper".into());
            }
            Ok(ir::PatternTestKind::IntRange { lo: low, hi: high })
        }
        _ => Err(format!("range pattern does not match {ty:?}")),
    }
}

/// `&dyn P` / `&mut dyn P` coercion target: (reference kind, principal).
pub(super) fn trait_object_target(ty: &ir::Ty) -> Option<(ir::BorrowKind, Arc<ir::DynTy>)> {
    let ir::Ty::Ref { kind, referent } = ty else {
        return None;
    };
    let ir::Ty::Dyn(object) = referent.as_ref() else {
        return None;
    };
    Some((*kind, object.clone()))
}

#[cfg(test)]
mod checked_identity_tests {
    #[test]
    fn direct_check_preserves_canonical_source_lowering() {
        for source in [
            "struct Wrap<T>{value:T}fn unwrap<T>(x:Wrap<T>)->T{x.value}fn main(){let x=unwrap::<i32>(Wrap{value:1});}",
            "struct D{v:i32}impl Drop for D{fn drop(&mut self){}}fn main(){let d=D{v:1};}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        let invalid = parse("fn duplicate<T>(x:T){let a=x;let b=x;}fn main(){}").unwrap();
        assert!(
            check(&invalid).is_err(),
            "unused generic moves must remain checked"
        );
    }

    #[test]
    fn guarded_matches_reject_noncopy_bindings_before_guard_checking() {
        for source in [
            "fn f<T>(x:T)->bool{matches!(x,y if true)}fn main(){}",
            "struct Token;fn consume(x:Token)->bool{true}fn main(){let value=Token;let result=matches!(value,bound if consume(bound));}",
            "struct Token;fn main(){let value=Token;let result=matches!(value,bound if true);}",
            "struct Token;struct Wrap{value:Token}fn main(){let value=Wrap{value:Token};let result=matches!(value,Wrap{value:bound} if false);}",
            "struct Token;fn main(){if false{let value=Token;let result=matches!(value,bound if true);}}",
        ] {
            let program = parse(source).unwrap();
            let error = check(&program).unwrap_err();
            assert!(
                error.contains(
                    "unsupported: guarded matches with non-Copy bindings are unsupported"
                ),
                "{source}: {error}"
            );
        }
    }

    #[test]
    fn guarded_matches_reject_mutable_bindings_before_guard_checking() {
        for source in [
            "fn main(){let value=1;let result=matches!(value,mut bound if {bound=2;true});}",
            "fn main(){let value=(1,2);let result=matches!(value,(mut bound,_) if false);}",
            "fn main(){if false{let value=1;let result=matches!(value,mut bound if true);}}",
        ] {
            let program = parse(source).unwrap();
            let error = check(&program).unwrap_err();
            assert!(
                error
                    .contains("unsupported: guarded matches with mutable bindings are unsupported"),
                "{source}: {error}"
            );
        }
    }

    #[test]
    fn guarded_matches_admit_copy_bindings_and_binding_free_noncopy_values() {
        for source in [
            "fn f<T:Copy>(x:T)->bool{matches!(x,y if true)}fn main(){}",
            "fn main(){let value=1;let result=matches!(value,bound if bound==1);}",
            "struct Token;fn main(){let value=Token;let result=matches!(value,_ if true);}",
            "struct Token;struct Wrap{value:Token}fn main(){let value=Wrap{value:Token};let result=matches!(value,Wrap{value:_} if true);}",
            "struct Token;struct Wrap{token:Token,number:i32}fn main(){let value=Wrap{token:Token,number:1};let result=matches!(value,Wrap{token:_,number:bound} if bound==1);}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn closed_associated_types_reach_checker_as_normalized_source_types() {
        for source in [
            "trait Project{type Out;}struct ProjectionOwner;struct ProjectionOther;impl Project for ProjectionOwner{type Out=i128;}fn pass(x:<ProjectionOwner as Project>::Out)-><ProjectionOwner as Project>::Out{x}fn main(){let value:<ProjectionOwner as Project>::Out=42i128;let result=pass(value);}",
            "trait Project{type Out;}struct ProjectionOwner;struct ProjectionOther;struct Cell{value:i32}impl Project for ProjectionOwner{type Out=Cell;}fn pass(x:<ProjectionOwner as Project>::Out)-><ProjectionOwner as Project>::Out{x}fn main(){let value:<ProjectionOwner as Project>::Out=Cell{value:42};let result=pass(value);}",
            "trait Project{type Out;}struct ProjectionOwner;struct ProjectionOther;impl Project for ProjectionOwner{type Out=(i128,bool);}type Output=<ProjectionOwner as Project>::Out;fn pass(x:Output)->Output{x}fn main(){let value:Output=(42i128,true);let result=pass(value);}",
            "trait Project{type Out;}struct ProjectionOwner;struct ProjectionOther;impl Project for ProjectionOwner{type Out=<ProjectionOther as Project>::Out;}impl Project for ProjectionOther{type Out=i128;}fn value()-><ProjectionOwner as Project>::Out{42i128}fn identity<T>(x:T)->T{x}fn main(){let result=identity(value());}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn required_projected_signature_is_a_nonexecuting_schema() {
        let source = "trait Project{type Out;fn value(x:Self::Out)->Self::Out;}fn main(){}";
        let program = parse(source).unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn raw_projected_contract_is_retained_until_exact_owner_normalization() {
        let program = parse("trait Project{type Out;fn value(x:Self::Out)->Self::Out;}struct S;impl Project for S{type Out=i128;fn value(x:Self::Out)->Self::Out{x}}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let implementation = world
            .impl_headers()
            .iter()
            .find(|header| !header.associated_types.is_empty())
            .unwrap();
        let (required, provided) = implementation.items[0];
        for declaration in [required, provided] {
            let raw = plan.raw_schema(declaration).unwrap();
            assert!(matches!(raw.signature.result, ir::Ty::AssociatedType(_)));
            assert!(raw.signature.ref_contract.input_regions.is_empty());
            assert!(raw.signature.ref_contract.results.is_empty());
            let carriers = ir::carrier_locations(&raw.signature.result, &budget).unwrap();
            let ir::Ty::AssociatedType(expected) = &raw.signature.result else {
                panic!("source projection was normalized prematurely");
            };
            assert_eq!(carriers.len(), 1);
            assert!(carriers[0].path.0.is_empty());
            assert!(
                matches!(&carriers[0].channel, ir::CarrierChannel::ProjectionTail { projection, prefix: 0 } if projection == expected)
            );
        }
        let normalized = world.normalized_callable_schema(provided).unwrap();
        assert_eq!(normalized.signature.result, ir::Ty::Int(ir::IntTy::I128));
        assert!(normalized.signature.ref_contract.results.is_empty());
    }

    #[test]
    fn projected_defaults_and_rigid_callers_preserve_exact_projection() {
        for source in [
            "trait Project{type Out;fn value(x:Self::Out)->Self::Out{x}}fn main(){}",
            "trait Project{type Out;}fn pass<T:Project>(x:<T as Project>::Out)-><T as Project>::Out{x}fn main(){}",
        ] {
            check(&parse(source).unwrap()).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        for source in [
            "trait Project{type Out;fn value(x:Self::Out)->Self::Out{x}}struct S;impl Project for S{type Out=i32;}fn main(){println!(\"{}\",<S as Project>::value(42));}",
            "trait Project{type Out;}struct S;impl Project for S{type Out=i32;}fn pass<T:Project>(x:<T as Project>::Out)-><T as Project>::Out{x}fn main(){println!(\"{}\",pass::<S>(42));}",
        ] {
            super::super::parser_tests::stock_native_run(source);
        }
    }

    #[test]
    fn normalized_projection_does_not_invent_copy_or_accept_wrong_values() {
        for source in [
            "trait Project{type Out;}struct ProjectionOwner;struct ProjectionOther;struct Cell{value:i32}impl Project for ProjectionOwner{type Out=Cell;}fn copy_only<T:Copy>(x:T){}fn pass(x:<ProjectionOwner as Project>::Out){copy_only(x)}fn main(){}",
            "trait Project{type Out;}struct ProjectionOwner;struct ProjectionOther;impl Project for ProjectionOwner{type Out=i128;}fn pass(x:<ProjectionOwner as Project>::Out){}fn main(){pass(true);}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "{source}");
        }
    }

    #[test]
    fn unused_associated_assignment_is_not_hidden_by_checker_normalization() {
        let source = "trait Project{type Out;}struct ProjectionOwner;struct ProjectionOther;impl Project for ProjectionOwner{type Out=<ProjectionOther as Project>::Out;}fn main(){}";
        let program = parse(source).unwrap();
        assert!(check(&program).is_err());
    }

    #[test]
    fn projection_source_head_cannot_be_replaced_by_its_equal_normal_form() {
        let source = "trait Project{type Out;}struct ProjectionOwner;struct ProjectionOther;impl Project for ProjectionOwner{type Out=i128;}fn value()-><ProjectionOwner as Project>::Out{42i128}fn main(){}";
        let original = parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&original, &budget).unwrap();
        let mut supplied = original.clone();
        supplied.functions[0].result = Type::Int(ir::IntTy::I128);
        let error = type_function_with_plan(&supplied, 0, &plan, &budget).unwrap_err();
        assert!(error.contains("immutable planning declaration"), "{error}");
    }

    #[test]
    fn typed_slices_check_exact_wide_and_aggregate_elements() {
        for source in [
            "fn read(s:&[i128])->i128{s[0]}fn main(){let values=[42i128,7i128];let answer=read(&values);}",
            "fn read<T:Copy>(s:&[T])->T{s[0]}fn main(){let values=[42u16,7u16];let answer=read(&values);}",
            "fn read<T:Copy>(s:&[T])->T{s[0]}fn forward<T:Copy>(s:&[T])->T{read(s)}fn main(){}",
            "fn read(s:&[(i32,bool)])->i32{s[0].0}fn main(){let values=[(42,true)];let answer=read(&values);}",
            "struct Cell{n:i32}fn length(s:&[Cell])->usize{s.len()}fn main(){let values=[Cell{n:42}];let count=length(&values);}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn typed_slice_inference_types_temporary_array_borrows() {
        for source in [
            "fn read<T:Copy>(s:&[T])->T{s[0]}fn value()->u16{read(&[42])}fn main(){}",
            "fn read<T:Copy>(s:&[T],later:T)->T{s[0]}fn main(){let answer=read(&[42],7u16);}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
    }

    #[test]
    fn typed_slice_reads_do_not_invent_noncopy_borrowed_places() {
        for source in [
            "struct Cell{n:i32}fn read(s:&[Cell])->Cell{s[0]}fn main(){}",
            "fn read<T>(s:&[T])->T{s[0]}fn main(){}",
            "struct Cell{n:i32}fn read(s:&[Cell])->i32{s[true].n}fn main(){}",
            "struct Cell{n:i32}fn read(s:&[Cell])->i32{s[0].missing}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let error = check(&program).unwrap_err();
            assert!(!error.contains("unsupported:"), "{source}: {error}");
        }
        for source in [
            "struct Cell{n:i32}fn read(s:&[Cell])->i32{s[0].n}fn main(){}",
            "struct Inner{n:i32}struct Cell{inner:Inner}fn read(s:&[Cell])->i32{s[0].inner.n}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let error = check(&program).unwrap_err();
            assert!(
                error.contains(
                    "borrowed field reads through non-Copy slice elements are unsupported"
                ),
                "{error}"
            );
        }
    }

    #[test]
    fn typed_slice_unsizing_keeps_exact_element_and_reference_boundaries() {
        for source in [
            "fn read(s:&[u16])->u16{s[0]}fn main(){let values=[42u8];let answer=read(&values);}",
            "fn read(s:&[i32])->i32{s[0]}fn main(){let values=[true];let answer=read(&values);}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "{source}");
        }
        let source = "fn read<T:Copy>(s:&[T])->T{s[0]}fn main(){let n=42;let values=[&n];let answer=read(&values);}";
        let program = parse(source).unwrap();
        assert!(check(&program).is_err());
    }
    use super::*;

    const CALL_PLACE_SOURCE: &str = "struct In{v:i32,w:[i32;2]}struct S{a:In,t:(i32,In)}fn p(s:&mut S)->&mut S{s}fn q(s:&S)->&S{s}fn main(){let mut s=S{a:In{v:0,w:[0;2]},t:(0,In{v:0,w:[0;2]})};p(&mut s).a.v=1;(*p(&mut s)).a.w[1]+=2;p(&mut s).zzz=3;p(&mut s).a=In{v:0,w:[0;2]};}";

    fn call_place_fixture() -> (Program, Signatures) {
        let program = parse(CALL_PLACE_SOURCE).unwrap();
        let signatures = signature_table(&program).unwrap();
        (program, signatures)
    }
    /// Result type of `p` (`&mut S`) or `q` (`&S`) from the planned schema.
    fn carrier(signatures: &Signatures, function: usize) -> ir::Ty {
        let plan = signatures.plan().unwrap();
        let declaration = plan.function_decl(function).unwrap();
        plan.function_schema(declaration).unwrap().signature.result
    }
    /// Written selectors of the `n`th call-rooted assignment in `main`.
    fn written(program: &Program, n: usize) -> Vec<FieldSelector> {
        program.functions[2]
            .body
            .statements
            .iter()
            .filter_map(|statement| match statement {
                Stmt::AssignDeref { fields, .. } => Some(fields.clone()),
                _ => None,
            })
            .nth(n)
            .unwrap()
    }
    fn budget_of(signatures: &Signatures) -> &ir::PlanBudget {
        &signatures.plan().unwrap().budget
    }

    #[test]
    fn call_place_projection_matches_checked_place_rules() {
        let (program, signatures) = call_place_fixture();
        let checker = Checker::new(&signatures, ir::Ty::Unit, program.functions[2].module);
        let mutable = carrier(&signatures, 0);
        let shared = carrier(&signatures, 1);
        // p().a.v : implicit Deref, Field(a), Field(v)
        let av = written(&program, 0);
        let (projection, ty, writable) = checker.call_place_projection(&mutable, &av).unwrap();
        assert_eq!(
            projection,
            vec![
                ir::Projection::Deref,
                ir::Projection::Field(0),
                ir::Projection::Field(0)
            ]
        );
        assert_eq!(ty, ir::Ty::Int(ir::IntTy::I32));
        assert!(writable);
        assert_eq!(projection.capacity(), projection.len());
        // (*p()).a.w[1] : explicit Deref, Field, Field, ConstantIndex
        let (projection, ty, _) = checker
            .call_place_projection(&mutable, &written(&program, 1))
            .unwrap();
        assert_eq!(
            projection,
            vec![
                ir::Projection::Deref,
                ir::Projection::Field(0),
                ir::Projection::Field(1),
                ir::Projection::ConstantIndex(1)
            ]
        );
        assert_eq!(ty, ir::Ty::Int(ir::IntTy::I32));
        // Shared carrier: never writable; the hidden binding grants nothing.
        let (_, _, writable) = checker.call_place_projection(&shared, &av).unwrap();
        assert!(!writable);
        // Unknown field keeps the base diagnostic.
        assert!(
            checker
                .call_place_projection(&mutable, &written(&program, 2))
                .is_err()
        );
        // Aggregate destination.
        let (projection, ty, _) = checker
            .call_place_projection(&mutable, &written(&program, 3))
            .unwrap();
        assert_eq!(projection.len(), 2);
        assert!(matches!(ty, ir::Ty::Struct(_)));
    }

    #[test]
    fn call_place_projection_charges_before_allocation_and_stops_after_failure() {
        let (program, signatures) = call_place_fixture();
        let checker = Checker::new(&signatures, ir::Ty::Unit, program.functions[2].module);
        let mutable = carrier(&signatures, 0);
        let fields = written(&program, 0);
        // Warm the per-struct field memo so the measured delta is ours only.
        checker.call_place_projection(&mutable, &fields).unwrap();
        let before = budget_of(&signatures).usage_snapshot();
        let (projection, _, _) = checker.call_place_projection(&mutable, &fields).unwrap();
        let after = budget_of(&signatures).usage_snapshot();
        let paid = after.bytes - before.bytes;
        let element = std::mem::size_of::<ir::Projection>();
        // Requested bytes plus any measured capacity surplus are retained,
        // each through one existing PlanState reservation whose fixed
        // bookkeeping overhead is defined in nominal_schema::retain.
        let overhead = 4 * std::mem::size_of::<ir::PlanReservation>() + 64;
        assert_eq!(projection.capacity(), projection.len());
        assert_eq!(paid, projection.len() * element + overhead);
        assert!(after.work > before.work);
        // Once a planning failure is recorded, no further allocation happens.
        let limited = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(limited.charge_work(1).is_err());
        budget_of(&signatures).record_resource_failure(limited.first_resource_failure().unwrap());
        let failed = budget_of(&signatures).usage_snapshot();
        let error = checker
            .call_place_projection(&mutable, &fields)
            .unwrap_err();
        assert!(error.contains("already exhausted"), "{error}");
        assert_eq!(budget_of(&signatures).usage_snapshot().bytes, failed.bytes);
        // The original first failure is preserved.
        assert_eq!(
            budget_of(&signatures).first_resource_failure(),
            failed.first_failure
        );
    }

    #[test]
    fn constructor_method_receiver_fallback_preserves_explicit_and_local_widths() {
        for source in [
            "struct Wrap<T>(T);trait Convert<T>{fn convert(self)->T;}impl<T> Convert<T> for Wrap<T>{fn convert(self)->T{let Wrap(x)=self;x}}fn value()->u8{Wrap(42u8).convert()}fn main(){}",
            "struct Wrap<T>(T);trait Convert<T>{fn convert(self)->T;}impl<T> Convert<T> for Wrap<T>{fn convert(self)->T{let Wrap(x)=self;x}}fn value()->u8{let x:Wrap<u8>=Wrap(42);x.convert()}fn main(){}",
            "struct Wrap<T>(T);trait Convert<T>{fn convert(self)->T;}impl<T> Convert<T> for Wrap<T>{fn convert(self)->T{let Wrap(x)=self;x}}fn value()->u8{Wrap::<u8>(42).convert()}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn constructor_method_receiver_hint_never_evaluates_its_operand() {
        let program = parse(r#"struct Wrap<T>(T);trait Convert<T>{fn convert(self)->T;}impl<T> Convert<T> for Wrap<T>{fn convert(self)->T{let Wrap(x)=self;x}}fn tick()->i32{println!("once");42}fn value()->i32{Wrap(tick()).convert()}fn main(){}"#).unwrap();
        let signatures = signature_table(&program).unwrap();
        let function = &program.functions[1];
        let Some(Expr::MethodCall { receiver, .. }) = function.body.tail.as_deref() else {
            panic!("genuine method receiver");
        };
        let checker = Checker::new(&signatures, ir::Ty::Int(ir::IntTy::I32), function.module);
        assert!(matches!(
            checker.method_receiver_hint(receiver).unwrap(),
            Some(ir::Ty::Struct(_))
        ));
        assert!(checker.locals.is_empty());
        assert!(checker.exprs.is_empty());
        let typed = type_function_with_signatures(&program, 1, &signatures).unwrap();
        assert_eq!(
            typed
                .exprs
                .iter()
                .filter(|expression| matches!(expression.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn equality_required_calls_keep_rigid_trait_item_identity() {
        let program = parse("trait Source{type Out;fn get(self)->Self::Out;}fn read<T:Source<Out=i32>>(x:T)->i32{x.get()}fn main(){}").unwrap();
        let signatures = signature_table(&program).unwrap();
        let typed = type_function_with_signatures(&program, 0, &signatures).unwrap();
        let calls = typed
            .exprs
            .iter()
            .filter_map(|expression| match &expression.kind {
                ir::ExprKind::Call {
                    target, signature, ..
                } => Some((target, signature)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        let (ir::Callee::TraitItem(call), signature) = calls[0] else {
            panic!("rigid equality cannot select an implementation target");
        };
        assert!(matches!(call.trait_ref.self_ty, ir::Ty::Param(_)));
        assert_eq!(signature.result, ir::Ty::Int(ir::IntTy::I32));
        assert!(matches!(signature.params.as_slice(), [ir::Ty::Param(_)]));
    }

    #[test]
    fn equality_requirements_survive_unused_and_body_only_signatures() {
        for source in [
            "trait Source{type Out;}struct S;impl Source for S{type Out=i32;}fn unused<T:Source<Out=i32>>(x:T)->i32{42}fn main(){let n=unused(S);}",
            "trait Source{type Out;fn get(self)->Self::Out;}fn first<T:Source<Out=i32>>(x:T)->i32{x.get()}fn second<T:Source<Out=i32>>(x:T)->i32{first(x)}fn main(){}",
            "trait Source{type Out;fn get(self)->Self::Out;}fn read<T:Source<Out=i32>>(x:T)->i32{let n:<T as Source>::Out=x.get();n}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        for source in [
            "trait Source{type Out;}struct S;impl Source for S{type Out=bool;}fn unused<T:Source<Out=i32>>(x:T)->i32{42}fn main(){let n=unused(S);}",
            "trait Source{type Out;fn get(self)->Self::Out;}fn first<T:Source<Out=i32>>(x:T)->i32{x.get()}fn second<T:Source>(x:T)->i32{first(x)}fn main(){}",
            "trait Source{type Out;fn get(self)->Self::Out;}fn wrong<T:Source<Out=i32>>(x:T)->bool{x.get()}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn equality_source_replay_keeps_every_binding_occurrence() {
        let program = parse(
            "trait Source{type Out;}fn read<T:Source<Out=i32,Out=i32>>(x:T)->i32{42}fn main(){}",
        )
        .unwrap();
        let signatures = signature_table(&program).unwrap();
        let plan = signatures.plan().unwrap();
        let original = &program.functions[0];
        assert!(signature_source_equal(plan, original.into(), original.into()).unwrap());
        let mut changed = original.clone();
        let TraitPath::Source(path) = &mut changed.bounds[0].traits[0] else {
            panic!("source equality bound");
        };
        path.segments.last_mut().unwrap().associated_types[1].value = Type::Bool;
        assert!(!signature_source_equal(plan, original.into(), (&changed).into()).unwrap());
        let TraitPath::Source(path) = &mut changed.bounds[0].traits[0] else {
            unreachable!();
        };
        path.segments.last_mut().unwrap().associated_types.pop();
        assert!(!signature_source_equal(plan, original.into(), (&changed).into()).unwrap());
    }

    #[test]
    fn equality_source_replay_checks_later_handles_before_comparison() {
        let source =
            "trait Source{type Out;}fn read<T:Source<Out=i32,Out=i32>>(x:T)->i32{42}fn main(){}";
        let program = parse(source).unwrap();
        let foreign = parse(source).unwrap();
        let signatures = signature_table(&program).unwrap();
        let plan = signatures.plan().unwrap();
        let original = &program.functions[0];
        let mut changed = original.clone();
        let TraitPath::Source(foreign_path) = &foreign.functions[0].bounds[0].traits[0] else {
            panic!("foreign source equality bound");
        };
        let TraitPath::Source(path) = &mut changed.bounds[0].traits[0] else {
            panic!("source equality bound");
        };
        let bindings = &mut path.segments.last_mut().unwrap().associated_types;
        bindings[0].value = Type::Bool;
        bindings[1].name = foreign_path.segments.last().unwrap().associated_types[1].name;
        assert!(signature_source_equal(plan, original.into(), (&changed).into()).is_err());
    }

    #[test]
    fn equality_source_replay_rejects_same_owner_occurrence_transplants() {
        let program = parse("struct Cell;trait Source{type Out;}fn first<T:Source<Out=Cell>>(x:T){}fn second<T:Source<Out=Cell>>(x:T){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        type_function_with_plan(&program, 0, &plan, &budget).unwrap();
        let TraitPath::Source(donor) = &program.functions[1].bounds[0].traits[0] else {
            panic!("donor bound");
        };
        let donor = &donor.segments.last().unwrap().associated_types[0];
        for transplant in 0..3 {
            let mut supplied = program.clone();
            let TraitPath::Source(path) = &mut supplied.functions[0].bounds[0].traits[0] else {
                panic!("recipient bound");
            };
            let binding = &mut path.segments.last_mut().unwrap().associated_types[0];
            match transplant {
                0 => *binding = donor.clone(),
                1 => binding.name = donor.name,
                _ => binding.value = donor.value.clone(),
            }
            assert!(
                type_function_with_plan(&supplied, 0, &plan, &budget).is_err(),
                "accepted same-owner transplant {transplant}"
            );
        }
    }

    #[test]
    fn immutable_function_head_equality_keeps_trait_application_actuals() {
        let program = parse("trait Convert<T>{fn convert(self)->T;}fn run<T:Convert<i32>>(value:T)->i32{value.convert()}fn main(){}").unwrap();
        let signatures = signature_table(&program).unwrap();
        let plan = signatures.plan().unwrap();
        let original = &program.functions[0];
        let mut changed = original.clone();
        let TraitPath::Source(path) = &mut changed.bounds[0].traits[0] else {
            panic!("source trait bound");
        };
        let argument = path.segments.last_mut().unwrap().args.first_mut().unwrap();
        *argument = Type::Bool;
        assert!(!signature_source_equal(plan, original.into(), (&changed).into()).unwrap());
        assert!(signature_source_equal(plan, original.into(), original.into()).unwrap());
    }

    #[test]
    fn exact_product_coverage_proves_bool_enum_array_and_record_partitions() {
        for source in [
            "struct Pair(bool,bool);fn value(v:Pair)->i32{match v{Pair(true,_)=>1,Pair(false,true)=>2,Pair(false,false)=>3}}fn main(){}",
            "enum Inner{A(bool),B}enum Outer{Some(Inner),None}fn value(v:Outer)->i32{match v{Outer::Some(Inner::A(true))=>1,Outer::Some(Inner::A(false))=>2,Outer::Some(Inner::B)=>3,Outer::None=>4}}fn main(){}",
            "fn value(v:[bool;2])->i32{match v{[true,_]=>1,[false,true]=>2,[false,false]=>3}}fn main(){}",
            "struct Pair{x:bool,y:bool}fn value(v:Pair)->i32{match v{Pair{x:true,..}=>1,Pair{x:false,y:true}=>2,Pair{x:false,y:false}=>3}}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn exact_product_holes_and_integer_complement_remain_invalid() {
        for source in [
            "struct Pair(bool,bool);fn value(v:Pair)->i32{match v{Pair(true,_)=>1,Pair(false,true)=>2}}fn main(){}",
            "enum E{A(bool),B}fn value(v:E)->i32{match v{E::A(true)=>1,E::B=>0}}fn main(){}",
            "struct Wide(i128);fn value(v:Wide)->i32{match v{Wide(-170141183460469231731687303715884105728i128)=>1,Wide(170141183460469231731687303715884105727i128)=>2}}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let error = check(&program).unwrap_err();
            assert!(error.contains("non-exhaustive"), "{source}: {error}");
            assert!(!error.contains("unsupported:"), "{source}: {error}");
        }
        let program = parse("struct Wide(i128);fn value(v:Wide)->i32{match v{Wide(-170141183460469231731687303715884105728i128)=>1,Wide(170141183460469231731687303715884105727i128)=>2,_=>3}}fn main(){}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn exact_subsumption_still_checks_dead_bodies_and_preserves_else_scope() {
        for source in [
            "struct Pair(bool,bool);fn value(v:Pair)->i32{match v{Pair(true,_)=>1,Pair(true,false)=>missing,Pair(false,_)=>2}}fn main(){}",
            "struct Pair(bool,bool);fn value(v:Pair)->i32{match v{Pair(true,_)=>1,Pair(true,false)=>false,Pair(false,_)=>2}}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            assert!(
                check(&program).is_err(),
                "accepted dead invalid body: {source}"
            );
        }
        let program = parse("enum E{A(bool,i32),B}fn value(v:E)->i32{let x=7;let E::A(true,x)=v else{return x;};x}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("enum E{A(bool,i32),B}fn value(v:E)->i32{let E::A(true,x)=v else{return x;};x}fn main(){}").unwrap();
        assert!(check(&program).is_err());
    }

    #[test]
    fn decision_patterns_preserve_ordered_tags_and_failed_arm_moves() {
        for source in [
            "enum Inner{A(bool,i32),B}enum Outer{Some(Inner),None}fn value(v:Outer)->i32{match v{Outer::Some(Inner::A(true,x))=>x,Outer::Some(_)=>0,Outer::None=>0}}fn main(){}",
            "struct Token;enum E{A(Token,bool),B}fn take(t:Token){}fn value(v:E){match v{E::A(t,true)=>take(t),E::A(t,_)=>take(t),E::B=>{}}}fn main(){}",
            "struct Token;struct Item{tag:bool,token:Token}fn value()->i32{let items=[Item{tag:true,token:Token}];match items{[Item{tag:true,..}]=>42,_=>0}}fn main(){}",
            "enum E{A(bool,i32),B}fn value(v:E)->i32{let E::A(true,x)=v else{return 0;};x}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn decision_patterns_require_proven_coverage_without_weakening_plain_let() {
        let program = parse("enum E{A(bool),B}fn value(v:E)->i32{match v{E::A(true)=>1,E::A(false)=>2,E::B=>0}}fn main(){}").unwrap();
        check(&program).unwrap();
        let program =
            parse("enum E{A(bool),B}fn value(v:E)->i32{match v{E::A(true)=>1}}fn main(){}")
                .unwrap();
        let error = check(&program).unwrap_err();
        assert!(error.contains("non-exhaustive"));
        assert!(!error.contains("unsupported:"));
        let program =
            parse("enum E{A(bool),B}fn value(v:E){let E::A(true)=v;}fn main(){}").unwrap();
        assert!(
            check(&program)
                .unwrap_err()
                .contains("unsupported: refutable enum patterns")
        );
        let program = parse("enum E{Only(i32)}struct Packet(E);fn value(v:Packet)->i32{match v{Packet(E::Only(x))=>x}}fn main(){}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn pending_decision_tests_hold_payload_leases_and_do_not_publish_early() {
        let program = parse(
            "enum E{A(bool,i32),B}fn value(v:E)->i32{match v{E::A(true,x)=>x,_=>0}}fn main(){}",
        )
        .unwrap();
        let function = &program.functions[0];
        let Expr::Match { arms, .. } = function.body.tail.as_deref().unwrap() else {
            panic!("expected match");
        };
        let pattern = &arms[0].pattern;
        let Pattern::Variant { name, .. } = pattern else {
            panic!("expected enum pattern");
        };
        let signatures = signature_table(&program).unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let (enumeration, _) = checker.enum_variant(name).unwrap();
        let budget = program.names.budget();
        let before = budget.snapshot().temporary_bytes;
        let plan = checker
            .collect_pattern(pattern, &ir::Ty::Enum(enumeration))
            .unwrap();
        let SourcePatternTest::Decision { tests, .. } = &plan.test else {
            panic!("expected decision plan");
        };
        assert_eq!(tests.len(), 2);
        assert!(tests.as_slice()[0].projection.is_empty());
        assert!(matches!(
            tests.as_slice()[0].kind,
            ir::PatternTestKind::EnumVariant { .. }
        ));
        assert!(matches!(
            tests.as_slice()[1].projection.as_slice(),
            [ir::Projection::Downcast(_), ir::Projection::Field(0)]
        ));
        assert_eq!(plan.bindings.rows.len(), 1);
        assert!(checker.locals.is_empty());
        assert!(budget.snapshot().temporary_bytes > before);
        drop(plan);
        assert_eq!(budget.snapshot().temporary_bytes, before);
    }

    #[test]
    fn root_or_pending_rows_hold_all_nested_scratch_until_drop() {
        use super::super::super::{expansion_budget::Storage, token_session::BudgetVec};
        let program =
            parse("fn value(p:(bool,u8))->u8{match p{(true,0)|(false,1)=>7,_=>0}}fn main(){}")
                .unwrap();
        let function = &program.functions[0];
        let Expr::Match { arms, .. } = function.body.tail.as_deref().unwrap() else {
            panic!("match")
        };
        let Pattern::Alternatives(rows) = &arms[0].pattern else {
            panic!("alternatives")
        };
        let signatures = signature_table(&program).unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let budget = program.names.budget();
        let before = budget.snapshot().temporary_bytes;
        let mut pending = BudgetVec::new(budget, Storage::Temporary).unwrap();
        let domain = ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Int(ir::IntTy::U8)].into());
        for row in rows {
            pending
                .push(checker.collect_pattern(row, &domain).unwrap())
                .unwrap();
        }
        assert_eq!(pending.len(), 2);
        assert!(
            budget.snapshot().temporary_bytes
                > before + (2 * std::mem::size_of::<PendingPatternPlan>()) as u64
        );
        assert!(checker.locals.is_empty());
        drop(pending);
        assert_eq!(budget.snapshot().temporary_bytes, before);
    }

    #[test]
    fn root_or_pending_header_denies_saturation_and_retry_without_publication() {
        use super::super::super::{
            expansion_budget::{BudgetResource, Storage},
            token_session::BudgetVec,
        };
        let program = parse("fn value()->u8{match true{true|false=>7}}fn main(){}").unwrap();
        let function = &program.functions[0];
        let Expr::Match { arms, .. } = function.body.tail.as_deref().unwrap() else {
            panic!("match")
        };
        let Pattern::Alternatives(rows) = &arms[0].pattern else {
            panic!("alternatives")
        };
        let signatures = signature_table(&program).unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let budget = program.names.budget();
        let mut pending = BudgetVec::new(budget, Storage::Temporary).unwrap();
        let row = checker.collect_pattern(&rows[0], &ir::Ty::Bool).unwrap();
        let before = budget.snapshot();
        let remaining = budget.limits().bytes - before.retained_bytes - before.temporary_bytes;
        let saturation = budget.reserve(Storage::Temporary, remaining).unwrap();
        assert!(pending.push(row).is_err());
        assert_eq!(pending.len(), 0);
        let failure = budget.snapshot().failure.unwrap();
        assert_eq!(failure.resource, BudgetResource::Bytes);
        assert_eq!(
            failure.requested,
            (4 * std::mem::size_of::<PendingPatternPlan>()) as u64
        );
        drop(saturation);
        assert!(checker.collect_pattern(&rows[1], &ir::Ty::Bool).is_err());
        assert!(checker.locals.is_empty());
        drop(pending);
        assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    }

    #[test]
    fn pending_decision_test_denies_real_row_growth_before_allocation_or_publication() {
        use super::super::super::expansion_budget::{BudgetFailureKind, BudgetResource, Storage};
        let program =
            parse("enum E{A(bool),B}fn value(v:E)->i32{match v{E::A(true)=>1,_=>0}}fn main(){}")
                .unwrap();
        let function = &program.functions[0];
        let Expr::Match { arms, .. } = function.body.tail.as_deref().unwrap() else {
            panic!("expected match");
        };
        let Pattern::Variant { name, .. } = &arms[0].pattern else {
            panic!("expected enum pattern");
        };
        let signatures = signature_table(&program).unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let (ty, variant) = checker.enum_variant(name).unwrap();
        let budget = program.names.budget();
        let mut pending = PendingDecision {
            tests: super::super::super::token_session::BudgetVec::new(budget, Storage::Temporary)
                .unwrap(),
            bindings: checker.pending_pattern_bindings().unwrap(),
            work: 0,
            refutable_payload: false,
        };
        let before = budget.snapshot();
        let remaining = budget.limits().bytes - before.retained_bytes - before.temporary_bytes;
        let saturation = budget.reserve(Storage::Temporary, remaining).unwrap();
        assert!(
            checker
                .decision_test(
                    &mut pending,
                    &[],
                    ir::PatternTestKind::EnumVariant { ty, variant }
                )
                .is_err()
        );
        let failed = budget.snapshot();
        let failure = failed.failure.as_ref().unwrap();
        assert_eq!(failure.kind, BudgetFailureKind::Limit);
        assert_eq!(failure.resource, BudgetResource::Bytes);
        assert_eq!(
            failure.requested,
            (4 * std::mem::size_of::<PendingDecisionTest>()) as u64
        );
        assert!(pending.tests.as_slice().is_empty());
        assert!(checker.locals.is_empty());
        assert_eq!(failed.temporary_bytes, before.temporary_bytes + remaining);
        drop(saturation);
        drop(pending);
        assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    }

    #[test]
    fn pending_pattern_plan_holds_metered_rows_without_publishing_locals() {
        let program = parse("fn value(){let x=1 else{return;};}fn main(){}").unwrap();
        let function = &program.functions[0];
        let Stmt::LetElse { pattern, .. } = &function.body.statements[0] else {
            panic!("expected let-else");
        };
        let Pattern::Binding { name, .. } = pattern.as_ref() else {
            panic!("expected binder");
        };
        let signatures = signature_table(&program).unwrap();
        let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let budget = program.names.budget();
        let before = budget.snapshot().temporary_bytes;
        let locals = checker.locals.len();
        let plan = checker
            .collect_pattern(pattern, &ir::Ty::Int(ir::IntTy::I32))
            .unwrap();
        assert_eq!(checker.locals.len(), locals);
        assert_eq!(checker.lookup_optional(*name).unwrap(), None);
        assert_eq!(plan.bindings.rows.len(), 1);
        assert!(
            budget.snapshot().temporary_bytes
                >= before + std::mem::size_of::<PendingPatternBinding>() as u64
        );
        let ir::MatchPattern::Binding(local) = checker.publish_pattern(plan).unwrap() else {
            panic!("expected published binding");
        };
        assert_eq!(checker.lookup(*name).unwrap(), local);
        assert_eq!(checker.locals.len(), locals + 1);
        assert_eq!(budget.snapshot().temporary_bytes, before);
    }

    #[test]
    fn pending_pattern_plan_denies_real_row_growth_before_any_local_publication() {
        use super::super::super::expansion_budget::{BudgetFailureKind, BudgetResource, Storage};
        let program = parse("fn value(){let x=1 else{return;};}fn main(){}").unwrap();
        let function = &program.functions[0];
        let Stmt::LetElse { pattern, .. } = &function.body.statements[0] else {
            panic!("expected let-else");
        };
        let signatures = signature_table(&program).unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let budget = program.names.budget();
        let before = budget.snapshot();
        let remaining = budget.limits().bytes - before.retained_bytes - before.temporary_bytes;
        let saturation = budget.reserve(Storage::Temporary, remaining).unwrap();
        assert!(
            checker
                .collect_pattern(pattern, &ir::Ty::Int(ir::IntTy::I32))
                .is_err()
        );
        assert!(checker.locals.is_empty());
        let failed = budget.snapshot();
        let failure = failed.failure.as_ref().unwrap();
        assert_eq!(failure.kind, BudgetFailureKind::Limit);
        assert_eq!(failure.resource, BudgetResource::Bytes);
        assert_eq!(
            failure.requested,
            (4 * std::mem::size_of::<PendingPatternBinding>()) as u64
        );
        assert_eq!(failed.temporary_bytes, before.temporary_bytes + remaining);
        drop(saturation);
        assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    }

    #[test]
    fn enum_rest_selection_retains_actual_backing_through_consumption_and_drop() {
        let program = parse("enum Item{Only(i32,i32,i32)}fn value(item:Item)->i32{match item{Item::Only(a,..,z)=>a+z}}fn main(){}").unwrap();
        let function = &program.functions[0];
        let Expr::Match { arms, .. } = function.body.tail.as_deref().unwrap() else {
            panic!("expected source match");
        };
        let pattern = &arms[0].pattern;
        let Pattern::VariantRest { name, .. } = pattern else {
            panic!("expected source enum rest");
        };
        let signatures = signature_table(&program).unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let (ty, _) = checker.enum_variant(name).unwrap();
        let budget = program.names.budget();
        let before = budget.snapshot().temporary_bytes;
        let (_, variant, fields) = checker.variant_pattern(pattern, &ir::Ty::Enum(ty)).unwrap();
        assert_eq!(variant, 0);
        assert!(matches!(&fields, SelectedVariantFields::Positional(_)));
        let held = budget.snapshot().temporary_bytes;
        assert!(held >= before + (2 * std::mem::size_of::<(usize, &Pattern)>()) as u64);
        let selected = fields.as_slice();
        assert_eq!(selected.len(), 2);
        assert_eq!((selected[0].0, selected[1].0), (0, 2));
        for (_, pattern) in selected {
            assert!(matches!(pattern, Pattern::Binding { .. }));
            assert_eq!(budget.snapshot().temporary_bytes, held);
        }
        drop(fields);
        assert_eq!(budget.snapshot().temporary_bytes, before);
    }

    #[test]
    fn enum_rest_selection_denies_bytes_before_backing_growth_and_cleans_up_after_poison() {
        use super::super::super::expansion_budget::{BudgetFailureKind, BudgetResource, Storage};
        let program = parse("enum Item{Only(i32,i32,i32)}fn value(item:Item)->i32{match item{Item::Only(a,..,z)=>a+z}}fn main(){}").unwrap();
        let function = &program.functions[0];
        let Expr::Match { arms, .. } = function.body.tail.as_deref().unwrap() else {
            panic!("expected source match");
        };
        let pattern = &arms[0].pattern;
        let Pattern::VariantRest { name, .. } = pattern else {
            panic!("expected source enum rest");
        };
        let signatures = signature_table(&program).unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let (ty, variant) = checker.enum_variant(name).unwrap();
        let definition = &ty.def().variants[variant];
        let Pattern::VariantRest { fields, .. } = pattern else {
            panic!("expected source enum rest");
        };
        let domain = ir::Ty::Enum(ty.clone());
        drop(checker.variant_pattern(pattern, &domain).unwrap());
        let budget = program.names.budget();
        let before = budget.snapshot();
        let remaining = budget.limits().bytes - before.retained_bytes - before.temporary_bytes;
        let saturation = budget.reserve(Storage::Temporary, remaining).unwrap();
        assert!(checker.select_enum_rest_fields(fields, definition).is_err());
        let failed = budget.snapshot();
        let failure = failed.failure.as_ref().unwrap();
        assert_eq!(failure.kind, BudgetFailureKind::Limit);
        assert_eq!(failure.resource, BudgetResource::Bytes);
        assert_eq!(failure.used, budget.limits().bytes);
        assert_eq!(
            failure.requested,
            (4 * std::mem::size_of::<(usize, &Pattern)>()) as u64
        );
        assert_eq!(failed.temporary_bytes, before.temporary_bytes + remaining);
        drop(saturation);
        assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    }

    #[test]
    fn enum_tuple_rest_preserves_variant_coverage_and_single_variant_let_rules() {
        let program = parse("enum Item{A(i32,i32,i32),B}fn value(item:Item)->i32{match item{Item::A(a,..,z)=>a+z,Item::B=>0}}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("enum Item{A(i32,i32,i32),B}fn value(item:Item)->i32{match item{Item::A(a,..,z)=>a+z}}fn main(){}").unwrap();
        assert!(check(&program).unwrap_err().contains("non-exhaustive"));
        let program = parse(
            "enum Item{A(i32,i32,i32),B}fn value(item:Item){let Item::A(a,..,z)=item;}fn main(){}",
        )
        .unwrap();
        assert!(
            check(&program)
                .unwrap_err()
                .contains("unsupported: refutable enum patterns")
        );
    }

    #[test]
    fn enum_tuple_rest_uses_existing_generic_and_zero_tuple_schemas() {
        let program = parse("enum Item<T>{Only(T,i32,T)}fn value()->i32{let item:Item<i32> = Item::Only(1,2,3);let Item::Only(a,..,z)=item;a+z}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse(
            "enum Item{Zero()}fn value()->i32{match Item::Zero(){Item::Zero(..)=>42}}fn main(){}",
        )
        .unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn enum_tuple_rest_moves_only_selected_noncopy_payloads() {
        let program = parse("struct Token;enum Item{Only(Token,Token,Token)}fn take(x:Token){}fn value(){let item=Item::Only(Token,Token,Token);match item{Item::Only(first,..)=>take(first)};match item{Item::Only(..,last)=>take(last)};}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("struct Token;enum Item{Only(Token,Token,Token)}fn take(x:Token){}fn value(){let item=Item::Only(Token,Token,Token);match item{Item::Only(first,..)=>take(first)};match item{Item::Only(first,..)=>take(first)};}fn main(){}").unwrap();
        assert!(check(&program).is_err());
    }

    #[test]
    fn array_patterns_use_real_indices_and_nested_copy_fields() {
        for source in [
            "fn value()->i32{let [first,..,last]=[1,2,3,4];first+last}fn main(){}",
            "fn value()->i32{match [[1,2],[3,4]]{[[a,_],[_,z]]=>a+z}}fn main(){}",
            "struct Token;struct Item{number:i32,token:Token}fn value()->i32{let a=[Item{number:42,token:Token}];let [Item{number,..}]=a;number}fn main(){}",
            "struct Token;fn take(a:[Token;2]){}fn value(){let a=[Token,Token];let [_,..]=a;take(a);}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap();
        }
    }

    #[test]
    fn let_else_publishes_real_bindings_only_after_the_old_scope_else() {
        for source in [
            "enum Item{Some(i32),None}fn value(item:Item)->i32{let x=7;let Item::Some(x)=item else{return x;};x}fn main(){}",
            "fn value()->i32{let x=7;let x=x+1 else{return x;};x}fn main(){}",
            "struct Token;enum Item{Some(Token),None}fn take(t:Token){}fn value(item:Item){let Item::Some(t)=item else{return;};take(t);}fn main(){}",
            "fn value()->i32{let true=(loop{}) else{return 7;};}fn main(){}",
            "fn value()->i32{return 7;let true=(loop{}) else{return 8;};}fn main(){}",
            "fn value()->i32{loop{let true=false else{break;};}42}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn let_else_checks_dead_else_and_requires_actual_never_type() {
        for source in [
            "enum Item{Some(i32),None}fn value(item:Item)->i32{let Item::Some(x)=item else{return x;};x}fn main(){}",
            "fn value(){let x=1 else{missing;};}fn main(){}",
            "fn value(){let true=false else{};}fn main(){}",
            "fn value(){let true=false else{while let true=(loop{}){}};}fn main(){}",
            "struct Token;enum Item{Some(Token),None}fn take(t:Token){}fn value(item:Item){let Item::Some(t)=item else{return;};take(t);take(t);}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn if_let_keeps_real_then_bindings_and_joins_actual_else_branches() {
        for source in [
            "enum Item{Some(i32),None}fn value(item:Item)->i32{let x=7;if let Item::Some(x)=item{x}else{x}}fn main(){}",
            "fn value()->i32{if let true=false{1}else if let 2=2{42}else{0}}fn main(){}",
            "fn value()->i32{if let [first,..,last]=[1,2,3]{first+last}else{0}}fn main(){}",
            "fn value()->i32{if let true=(loop{}){42}else{7}}fn main(){}",
            "fn value(){if let _=1{loop{}}}fn main(){}",
            "struct Token;enum Item{Some(Token),None}fn take(t:Token){}fn other(t:Item){}fn value(item:Item){if let Item::Some(t)=item{take(t);}else{other(item);}}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap();
        }
    }

    #[test]
    fn while_let_preserves_unit_obligation_and_actual_loop_completion() {
        let program = parse("fn value()->i32{while let _=1{};42}fn main(){}").unwrap();
        check(&program).unwrap();
        let signatures = signature_table(&program).unwrap();
        let checked = type_function_with_signatures(&program, 0, &signatures).unwrap();
        let loops = checked
            .exprs
            .iter()
            .filter(|expr| matches!(expr.kind, ir::ExprKind::Loop { .. }))
            .collect::<Vec<_>>();
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].ty, ir::Ty::Unit);
        assert_eq!(loops[0].completion, ir::Completion::Never);
        let program = parse("fn value()->i32{while let _=1{}}fn main(){}").unwrap();
        assert!(check(&program).is_err());
        let program =
            parse("fn value()->i32{while let true={return 7;}{let x=1;}8}fn main(){}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn while_let_never_condition_keeps_source_unit_and_intrinsic_exit_facts() {
        let program =
            parse("fn value()->i32{return 7;while let true=(loop{}){}}fn main(){}").unwrap();
        assert!(check(&program).is_err());
        let program =
            parse("fn value()->i32{return 7;while let true=(loop{}){};}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("fn value()->i32{while let true=(loop{}){}}fn main(){}").unwrap();
        assert!(check(&program).is_err());
        let program = parse("fn value(){while let true=(loop{}){}}fn main(){}").unwrap();
        check(&program).unwrap();
        let signatures = signature_table(&program).unwrap();
        let checked = type_function_with_signatures(&program, 0, &signatures).unwrap();
        let loops = checked
            .exprs
            .iter()
            .filter(|expr| matches!(expr.kind, ir::ExprKind::Loop { .. }))
            .collect::<Vec<_>>();
        assert_eq!(loops.len(), 2);
        let lowered = loops.last().unwrap();
        assert_eq!(lowered.ty, ir::Ty::Unit);
        assert_eq!(lowered.completion, ir::Completion::Never);

        let program = parse("fn value(){return;while let true=false{break;}}fn main(){}").unwrap();
        check(&program).unwrap();
        let signatures = signature_table(&program).unwrap();
        let checked = type_function_with_signatures(&program, 0, &signatures).unwrap();
        let loops = checked
            .exprs
            .iter()
            .filter(|expr| matches!(expr.kind, ir::ExprKind::Loop { .. }))
            .collect::<Vec<_>>();
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].ty, ir::Ty::Unit);
        assert_eq!(loops[0].completion, ir::Completion::Returns);
    }

    #[test]
    fn while_let_reinitializes_selected_noncopy_values_before_continue() {
        let program = parse("struct Token;enum Item{Some(Token),None}fn take(t:Token){}fn value(){let mut item=Item::Some(Token);while let Item::Some(t)=item{take(t);item=Item::None;continue;}}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("struct Token;enum Item{Some(Token),None}fn take(t:Token){}fn value(){let item=Item::Some(Token);while let Item::Some(t)=item{take(t);continue;}}fn main(){}").unwrap();
        assert!(check(&program).is_err());
        let program = parse("enum Item{Some(i32),None}fn value(item:Item)->i32{let x=7;while let Item::Some(x)=item{break;}x}fn main(){}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn while_let_keeps_condition_barrier_dead_body_and_binding_boundaries() {
        for source in [
            "fn value(){while let _=1{break 42;}}fn main(){}",
            "fn value(){loop{while let true={continue;}{}}}fn main(){}",
            "fn value(){while let true=false{42}}fn main(){}",
            "fn value(){while let true=(loop{}){missing;}}fn main(){}",
            "enum Item{Some(i32),None}fn value(item:Item)->i32{while let Item::Some(x)=item{break;}x}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn if_let_checks_dead_branches_missing_else_and_binding_escape() {
        for source in [
            "fn value(){if let x=1{42}}fn main(){}",
            "fn value()->i32{if let x=1{x}else{true}}fn main(){}",
            "fn value()->i32{if let true=false{missing}else{42}}fn main(){}",
            "enum Item{Some(i32),None}fn value(item:Item)->i32{if let Item::Some(x)=item{x}else{x}}fn main(){}",
            "fn value()->i32{if let x=1{};x}fn main(){}",
            "struct Token;enum Item{Some(Token),None}fn take(t:Token){}fn other(t:Item){}fn value(item:Item){if let Item::Some(t)=item{take(t);}other(item);}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn array_patterns_preserve_noncopy_and_shape_boundaries() {
        let program = parse("fn value()->i32{match [1,2]{[1,_]=>42,_=>0}}fn main(){}").unwrap();
        check(&program).unwrap();
        for source in [
            "struct Token;fn value(){let [item]=[Token];}fn main(){}",
            "struct Token;fn value(){match [Token]{[item]=>()};}fn main(){}",
            "fn value(){let [..]:[i32;0]=[];}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let error = check(&program).unwrap_err();
            assert!(error.contains("unsupported:"), "{error}");
        }
        for source in [
            "fn value(){let [a,b]=[1];}fn main(){}",
            "fn value(){let [a,b,..,z]=[1,2];}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err());
        }
    }

    #[test]
    fn positional_rest_maps_real_prefix_and_suffix_fields() {
        let program = parse("struct Pack(i32,i32,i32);fn value()->i32{let (first,..,last)=(1,2,3,4);match Pack(first,9,last){Pack(a,..,z)=>a+z}}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("fn value(){let (a,b,..,c)=(1,2);}fn main(){}").unwrap();
        assert!(check(&program).is_err());
    }

    #[test]
    fn positional_rest_on_unit_stays_typed_unsupported_in_match_and_let() {
        for source in [
            "fn value()->i32{match (){(..)=>42}}fn main(){}",
            "fn value(){let (..)=();}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let error = check(&program).unwrap_err();
            assert!(
                error.contains("unsupported: positional rest requires a nonempty tuple"),
                "{error}"
            );
        }
    }

    #[test]
    fn positional_rest_moves_selected_fields_without_moving_omitted_fields() {
        let program = parse("struct Token;struct Pack(Token,Token,Token);fn take(x:Token){}fn value(){let p=Pack(Token,Token,Token);match p{Pack(first,..)=>take(first)};take(p.2);}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("struct Token;struct Pack(Token,Token,Token);fn take(x:Token){}fn value(){let p=Pack(Token,Token,Token);match p{Pack(first,..)=>take(first)};take(p.0);}fn main(){}").unwrap();
        assert!(check(&program).is_err());
    }

    #[test]
    fn positional_rest_preserves_tuple_constructor_privacy_and_admits_enum_payloads() {
        assert!(parse("mod m{pub struct Pack(pub i32,i32,pub i32);pub fn make()->Pack{Pack(1,2,3)}}fn value()->i32{match m::make(){m::Pack(a,..,z)=>a+z}}fn main(){}").and_then(|program| check(&program)).is_err());
        let program = parse("enum Item{Only(i32,i32,i32)}fn value()->i32{match Item::Only(1,2,3){Item::Only(a,..,z)=>a+z}}fn main(){}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn nested_constructor_evidence_defers_fallback_until_outer_constraints() {
        for source in [
            r#"struct Wrap<T>(T);fn unwrap<T>(w:Wrap<T>)->T{let Wrap(x)=w;x}fn main(){if unwrap(Wrap(42))==42{println!("ok");}}"#,
            "struct Wrap<T>(T);fn unwrap<T>(w:Wrap<T>)->T{let Wrap(x)=w;x}fn value()->u8{unwrap(Wrap(42))}fn main(){}",
            "struct Wrap<T>(T);fn pick<T>(w:Wrap<T>,value:T)->T{value}fn main(){let x=pick(Wrap(42),1u8);}",
            "struct Wrap<T>(T);fn pick<T>(w:Wrap<T>,value:T)->T{value}fn main(){let x=pick(Wrap(2147483648),0i64);}",
            "struct Pair<T>(T,T);fn first<T>(p:Pair<T>)->T{let Pair(x,_)=p;x}fn main(){let x=first(Pair(42,1u8));}",
            "struct Wrap<T>(T);fn take<T>(w:Wrap<Wrap<T>>)->T{let Wrap(Wrap(x))=w;x}fn main(){let x=take(Wrap(Wrap(42)));}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        for source in [
            "struct Wrap<T>(T);fn pick<T>(w:Wrap<T>,value:T)->T{value}fn main(){let x=pick(Wrap(true),1i32);}",
            "struct Pair<T>(T,T);fn first<T>(p:Pair<T>)->T{let Pair(x,_)=p;x}fn main(){let x=first(Pair(1u8,2i32));}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn constructor_evidence_is_slot_free_and_does_not_evaluate_operands() {
        let program = parse(r#"struct Wrap<T>(T);fn tick()->i32{println!("once");42}fn unwrap<T>(w:Wrap<T>)->T{let Wrap(x)=w;x}fn main(){let x=unwrap(Wrap(tick()));}"#).unwrap();
        let signatures = signature_table(&program).unwrap();
        let function = &program.functions[2];
        let Stmt::Let {
            value: Expr::Call { args, .. },
            ..
        } = &function.body.statements[0]
        else {
            panic!("expected genuine call");
        };
        let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        let evidence = checker.inference_evidence(&args[0], 0).unwrap();
        checker.check_slot_free_evidence(&evidence, 0).unwrap();
        assert!(
            matches!(evidence.as_ref(), InferKind::Nominal(_, terms) if matches!(terms[0].as_ref(), InferKind::Rigid(ir::Ty::Int(ir::IntTy::I32))))
        );
        assert!(checker.locals.is_empty());
        assert!(checker.exprs.is_empty());
        let typed = type_function_with_signatures(&program, 2, &signatures).unwrap();
        assert_eq!(
            typed
                .exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
        let escaped = checker.infer_node(InferKind::Slot(0)).unwrap();
        assert!(
            checker
                .check_slot_free_evidence(&escaped, 0)
                .unwrap_err()
                .contains("contract:")
        );
    }

    #[test]
    fn generic_tuple_structs_use_existing_actuals_inference_and_pattern_domains() {
        for source in [
            "struct Wrap<T>(T);fn unwrap<T>(w:Wrap<T>)->T{let Wrap(x)=w;x}fn value()->i32{unwrap(Wrap(42))}fn main(){}",
            "struct Pair<T,U>(T,U);fn value()->i32{let pair:Pair<i32,bool> = Pair(42,true);let Pair(number,flag)=pair;if flag{number}else{0}}fn main(){}",
            "struct Pair<T,U>(T,U);fn value()->i32{let Pair(a,..,z)=Pair::<i32,i32>(40,2);a+z}fn main(){}",
            "struct Wrap<T>(T);fn value<T>(w:Wrap<T>)->T{match w{Wrap(x)=>x}}fn main(){}",
            "struct Wrap<T>(T);fn value<T>(w:Wrap<T>)->T{let Wrap(x)=w else{loop{}};x}fn main(){}",
            "struct Wrap<T>(T);fn value()->i32{let [Wrap(x)]=[Wrap(42)];x}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn generic_tuple_structs_preserve_universal_moves_and_actual_type_separation() {
        for source in [
            "struct Wrap<T>(T);fn duplicate<T>(w:Wrap<T>)->T{let Wrap(x)=w;let y=x;x}fn main(){let result=duplicate(Wrap(42));}",
            "struct Wrap<T>(T);fn value(){let wrong:Wrap<i32> = Wrap(true);}fn main(){}",
            "struct Pair<T>(T,T);fn value(){let wrong=Pair(1,true);}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            assert!(check(&program).is_err(), "accepted {source}");
        }
        let source = "struct Wrap<T>(T);fn value(){let x=Wrap::<i32>();}fn main(){}";
        let rejected = match parse(source) {
            Ok(program) => check(&program).is_err(),
            Err(_) => true,
        };
        assert!(rejected, "accepted {source}");
    }

    #[test]
    fn tuple_struct_let_patterns_and_numeric_fields_remain_nominal() {
        let program = parse("struct Pair(i32,bool);fn value()->i32{let p=Pair(7,true);let n=p.0;let Pair(x,_)=p;x+n}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("struct Pair(i32,bool);struct Other(i32,bool);fn value(){let Other(x,_)=Pair(7,true);}fn main(){}").unwrap();
        assert!(check(&program).is_err());
    }

    #[test]
    fn tuple_struct_match_destructuring_is_irrefutable() {
        let program = parse(
            "struct Pair(i32,bool);fn value()->i32{match Pair(7,true){Pair(x,_)=>x}}fn main(){}",
        )
        .unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn aggregate_match_patterns_validate_nested_shapes_and_reject_refutable_leaves() {
        let program = parse("struct Pair(i32,bool);struct Wrap{pair:Pair,tag:i32}fn value()->i32{let wrapped=Wrap{pair:Pair(7,true),tag:2};match wrapped{Wrap{pair:Pair(x,_),tag:n}=>x+n}}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("struct Pair(i32,bool);fn value()->i32{match Pair(7,true){Pair(7,_)=>1,_=>2}}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("struct Wrap{x:i32,y:i32}fn value()->i32{let wrapped=Wrap{x:1,y:2};match wrapped{Wrap{x,..}=>x}}fn main(){}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn record_rest_matches_omit_private_fields_but_check_selected_privacy() {
        let program = parse("mod m{pub struct Data{pub x:i32,hidden:i32}pub fn make()->Data{Data{x:7,hidden:2}}}fn value()->i32{match m::make(){m::Data{x,..}=>x}}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("mod m{pub struct Data{pub x:i32,hidden:i32}pub fn make()->Data{Data{x:7,hidden:2}}}fn value()->i32{match m::make(){m::Data{hidden,..}=>hidden}}fn main(){}").unwrap();
        assert!(check(&program).unwrap_err().contains("private field"));
    }

    #[test]
    fn record_rest_moves_only_selected_noncopy_fields() {
        let program = parse("struct Token;struct Wrap{selected:Token,other:Token}fn consume(x:Token){}fn value(){let wrapped=Wrap{selected:Token,other:Token};match wrapped{Wrap{selected,..}=>consume(selected)};consume(wrapped.other);}fn main(){}").unwrap();
        check(&program).unwrap();
        let program = parse("struct Token;struct Wrap{selected:Token,other:Token}fn consume(x:Token){}fn value(){let wrapped=Wrap{selected:Token,other:Token};match wrapped{Wrap{selected,..}=>consume(selected)};consume(wrapped.selected);}fn main(){}").unwrap();
        assert!(check(&program).is_err());
        let program = parse("struct Token;struct Wrap{selected:Token,other:Token}fn consume(x:Wrap){}fn value(){let wrapped=Wrap{selected:Token,other:Token};match wrapped{Wrap{..}=>{}}consume(wrapped);}fn main(){}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn unit_struct_patterns_preserve_nominal_domains() {
        let program =
            parse("struct U; fn value()->i32{let U=U;let U{}=U{};match U{U=>7}}fn main(){}")
                .unwrap();
        check(&program).unwrap();

        let program = parse("struct U;struct V;fn value()->i32{match U{V=>7}}fn main(){}").unwrap();
        assert!(check(&program).is_err());
    }

    #[test]
    fn unit_struct_transfer_remains_noncopy() {
        let program = parse(
            "struct U;fn consume(x:U){}fn value(){let x=U;consume(x);consume(x);}fn main(){}",
        )
        .unwrap();
        assert!(check(&program).is_err());
    }

    fn builder(name: &str) -> ProgramBuilder {
        ProgramBuilder::new(
            std::path::Path::new(name),
            "",
            &super::super::super::expansion_budget::ExpansionBudget::default(),
        )
        .unwrap()
    }

    #[test]
    fn anchored_local_references_retain_definition_frontier_across_shadowing() {
        use super::super::super::semantic_scope::ScopeArenaBuilder;
        let mut builder = builder("anchored-locals.rs");
        let main = builder.path("crate::main").unwrap();
        let module = builder.module_root().unwrap();
        builder.functions.push(Function {
            embedded_source: None,
            lexical_scope: None,
            module,
            constness: Constness::Runtime,
            unsafety: false,
            static_mut: false,
            visibility: ir::Visibility::Public,
            bounds: Vec::new(),
            name: main,
            regions: RegionDecls {
                receiver: None,
                params: Vec::new(),
                outlives: Vec::new(),
            },
            type_params: Vec::new(),
            params: Vec::new(),
            param_mut: Vec::new(),
            result: Type::Unit,
            impl_params: 0,
            body: Block {
                statements: Vec::new(),
                tail: None,
            },
        });
        let outer = builder.ident("x").unwrap();
        let inner = builder.ident("x").unwrap();
        let definition_reference = builder.ident("x").unwrap();
        let call_reference = builder.ident("x").unwrap();
        let unresolved = builder.ident("missing").unwrap();
        let unrecorded = builder.ident("x").unwrap();
        let foreign_function_reference = builder.ident("missing").unwrap();
        let mut scopes = ScopeArenaBuilder::for_names(builder.names()).unwrap();
        let root = scopes.function_root(builder.names()).unwrap();
        let function = scopes.function(root).unwrap();
        let (definition, outer_binding) = scopes.bind(builder.names(), root, outer).unwrap();
        let nested = scopes.enter(builder.names(), definition).unwrap();
        let (call, inner_binding) = scopes.bind(builder.names(), nested, inner).unwrap();
        assert_ne!(outer_binding, inner_binding);
        assert_eq!(
            scopes
                .resolve_reference(builder.names(), definition, definition_reference)
                .unwrap(),
            Some(outer_binding)
        );
        assert_eq!(
            scopes
                .resolve_reference(builder.names(), call, call_reference)
                .unwrap(),
            Some(inner_binding)
        );
        assert_eq!(
            scopes
                .resolve_reference(builder.names(), call, unresolved)
                .unwrap(),
            None
        );
        let other = scopes.function_root(builder.names()).unwrap();
        scopes
            .resolve_reference(builder.names(), other, foreign_function_reference)
            .unwrap();
        let arena = scopes.freeze().unwrap();
        let mut program = builder.finish().unwrap();
        program.scopes = Some(arena);
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let env = plan.environment(None).unwrap();
        let mut checker = Checker::new(&env, ir::Ty::Unit, program.modules.source_root().unwrap());
        checker.install_lexical_scope(Some(function)).unwrap();
        assert!(
            checker
                .lookup_optional(definition_reference)
                .unwrap_err()
                .contains("contract:")
        );
        assert!(
            checker
                .pending_reference(definition_reference)
                .unwrap_err()
                .contains("contract:")
        );
        let outer_local = checker.local(outer, ir::Ty::Bool, false).unwrap();
        checker.scopes.push(HashMap::new());
        let inner_local = checker.local(inner, ir::Ty::Bool, false).unwrap();
        assert_ne!(outer_local, inner_local);
        assert_eq!(checker.lookup(definition_reference).unwrap(), outer_local);
        assert_eq!(checker.lookup(call_reference).unwrap(), inner_local);
        assert_eq!(checker.lookup_optional(unresolved).unwrap(), None);
        assert!(
            checker
                .lookup_optional(unrecorded)
                .unwrap_err()
                .contains("contract:")
        );
        assert!(
            checker
                .lookup_optional(foreign_function_reference)
                .unwrap_err()
                .contains("contract:")
        );
    }

    #[test]
    fn generic_application_namespace_follows_expression_kind() {
        let mut builder = builder("application-namespace.rs");
        let name = builder.path("item").unwrap();
        let _program = builder.finish().unwrap();
        assert_eq!(
            application_namespace(&Expr::Call {
                name: name.clone(),
                args: Vec::new(),
            })
            .unwrap(),
            Namespace::Value
        );
        assert_eq!(
            application_namespace(&Expr::Struct {
                name,
                fields: Vec::new(),
            })
            .unwrap(),
            Namespace::Type
        );
        assert!(application_namespace(&Expr::Bool(false)).is_err());
    }

    #[test]
    fn checked_local_keys_join_distinct_occurrences_without_registering_wildcards() {
        let mut builder = builder("checked-locals.rs");
        let ordinary = builder.ident("value").unwrap();
        let repeated = builder.ident("value").unwrap();
        let path = builder.path("value").unwrap();
        assert!(builder.ident("r#value").is_err());
        let receiver = LocalName::Receiver;
        let program = builder.finish().unwrap();
        let names = &*program.names;
        assert_eq!(
            ordinary.checked_key(names).unwrap(),
            repeated.checked_key(names).unwrap()
        );
        assert_eq!(
            ordinary.checked_key(names).unwrap(),
            path.checked_key(names).unwrap()
        );
        assert_eq!(
            receiver.checked_key(names).unwrap(),
            Some(LocalKey::Receiver)
        );
        assert_eq!(BindingName::Wildcard.checked_key(names).unwrap(), None);
        assert_ne!(
            ordinary.checked_key(names).unwrap(),
            Some(LocalKey::Receiver)
        );
        assert_ne!(
            names.key(ordinary, LookupClass::Local).unwrap(),
            names.key(ordinary, LookupClass::Member).unwrap()
        );
    }

    #[test]
    fn foreign_local_occurrences_are_contract_errors_not_absent_locals() {
        let mut first = builder("checked-first.rs");
        let local = first.ident("value").unwrap();
        let program = first.finish().unwrap();
        let other = builder("checked-second.rs").finish().unwrap();
        assert!(local.checked_key(&other.names).is_err());
        assert!(local.checked_key(&program.names).is_ok());
    }

    #[test]
    fn active_callable_rejects_same_owner_signature_drift() {
        let original = parse("fn value(x:i32)->i32{x}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&original, &budget).unwrap();
        let mut supplied = original.clone();
        supplied.functions[0].result = Type::Bool;
        let error = type_function_with_plan(&supplied, 0, &plan, &budget).unwrap_err();
        assert!(error.contains("immutable planning declaration"), "{error}");
    }

    #[test]
    fn checking_one_active_callable_does_not_rescan_unrelated_mutated_bodies() {
        let original = parse("fn value()->i32{1}fn unused()->i32{2}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&original, &budget).unwrap();
        let mut supplied = original.clone();
        let mut foreign = builder("foreign-unrelated.rs");
        let name = foreign.path("foreign_value").unwrap();
        let _owner = foreign.finish().unwrap();
        supplied.functions[1].body.tail = Some(Box::new(Expr::Var(name)));
        assert!(type_function_with_plan(&supplied, 0, &plan, &budget).is_ok());
        assert!(type_function_with_plan(&supplied, 1, &plan, &budget).is_err());
    }
}
