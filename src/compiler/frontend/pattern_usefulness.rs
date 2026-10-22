//! Exact, sparse coverage of already typed conjunction rows.
//!
//! This module creates no bindings and grants no place/move authority. Partition
//! cells are symbolic, particularly the integer OTHER cell, not runtime values.
use super::super::{
    expansion_budget::{ByteLease, Counter, ExpansionBudget, Storage},
    ir::{self, FactsCtx, PatternTestKind, PlanBudget, Projection, Ty},
    source::{SourceError, SourceErrorKind},
    token_session::{BudgetVec, TokenError},
};

pub(super) struct TestRef<'row> {
    pub projection: &'row [Projection],
    pub kind: &'row PatternTestKind,
}

struct Test {
    projection: BudgetVec<Projection>,
    kind: PatternTestKind,
}
struct Row {
    tests: BudgetVec<Test>,
}
struct Rows {
    values: Vec<Row>,
    lease: ByteLease,
}
impl Rows {
    fn as_slice(&self) -> &[Row] {
        &self.values
    }
}

pub(super) struct CoverageMatrix<'facts> {
    // Payloads precede their accounting owners, including on poisoned exits.
    rows: Rows,
    domain: Ty,
    facts: FactsCtx<'facts>,
    plan: PlanBudget,
    expansion: ExpansionBudget,
}

fn token_error(error: TokenError) -> String {
    SourceError::from_token(error).to_string()
}
fn overflow() -> String {
    token_error(TokenError::IdExhausted)
}
fn temporary<T>(budget: &ExpansionBudget) -> Result<BudgetVec<T>, String> {
    BudgetVec::new(budget, Storage::Temporary).map_err(token_error)
}
fn push<T>(values: &mut BudgetVec<T>, value: T) -> Result<(), String> {
    values.push(value).map_err(token_error)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cell {
    Bool(bool),
    /// First ordinal of one elementary scalar segment. Every test interval on
    /// the split path either contains the whole segment or is disjoint from it.
    Scalar(u128),
    Variant(usize),
}
struct Assignment<'a> {
    test: &'a Test,
    value: Cell,
}
enum State<'a> {
    False,
    True,
    Open(&'a Test),
}
enum ChoiceKind {
    Bool,
    Enum(usize),
    Scalar,
}
struct Frame<'a> {
    // Segment starts, ascending. Backing is released before its lease by BudgetVec.
    integers: BudgetVec<u128>,
    test: &'a Test,
    kind: ChoiceKind,
    next: usize,
    base: usize,
}
impl Frame<'_> {
    fn next_cell(&mut self) -> Option<Cell> {
        let result = match self.kind {
            ChoiceKind::Bool if self.next < 2 => Some(Cell::Bool(self.next != 0)),
            ChoiceKind::Enum(count) if self.next < count => Some(Cell::Variant(self.next)),
            ChoiceKind::Scalar if self.next < self.integers.len() => {
                Some(Cell::Scalar(self.integers.as_slice()[self.next]))
            }
            _ => None,
        };
        if result.is_some() {
            // next is bounded by an existing vector length or the enum cap.
            self.next += 1;
        }
        result
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TypeIdentity(u8, usize);
enum DomainEvent<'a> {
    Enter(&'a Ty, usize),
    Leave,
}

impl<'facts> CoverageMatrix<'facts> {
    pub(super) fn new(
        domain: Ty,
        facts: FactsCtx<'facts>,
        plan: PlanBudget,
        expansion: ExpansionBudget,
    ) -> Result<Option<Self>, String> {
        let matrix = Self {
            rows: Rows {
                values: Vec::new(),
                lease: expansion
                    .reserve(Storage::Temporary, 0)
                    .map_err(|error| token_error(error.into()))?,
            },
            domain,
            facts,
            plan,
            expansion,
        };
        matrix.work(0)?;
        if matrix.applicable()? {
            Ok(Some(matrix))
        } else {
            // Only the explicit inhabitance boundary reaches this result.
            Ok(None)
        }
    }

    fn work(&self, count: usize) -> Result<(), String> {
        // PlanBudget records a rejected operation without rejecting all later
        // cheap charges. Coverage must not turn that prior failure into a
        // successful covered/exhaustive answer through a zero-work shortcut.
        if self.plan.has_resource_failure() {
            return Err("generic planning resource limit exceeded".into());
        }
        self.plan.charge_work(count)?;
        self.expansion
            .charge(Counter::Work, u64::try_from(count).map_err(|_| overflow())?)
            .map_err(|error| token_error(error.into()))
    }

    fn same_path(&self, a: &[Projection], b: &[Projection]) -> Result<bool, String> {
        self.work(a.len().max(b.len()).checked_add(1).ok_or_else(overflow)?)?;
        Ok(a == b)
    }

    /// Scan structural fields, not products of values. All variants and all
    /// retained by-value fields matter, even when no row mentions them. Generic
    /// parameters and references remain opaque as in the existing facts model.
    fn applicable(&self) -> Result<bool, String> {
        let mut pending = temporary(&self.expansion)?;
        let mut active = temporary(&self.expansion)?;
        push(&mut pending, DomainEvent::Enter(&self.domain, 0))?;
        let mut applicable = true;
        while let Some(event) = pending.pop() {
            self.work(1)?;
            let DomainEvent::Enter(ty, depth) = event else {
                active.pop().ok_or("coverage domain traversal imbalance")?;
                continue;
            };
            self.plan.check_depth(depth)?;
            let identity = match ty {
                Ty::Struct(value) => Some(TypeIdentity(0, value.def() as *const _ as usize)),
                Ty::Enum(value) => Some(TypeIdentity(1, value.def() as *const _ as usize)),
                Ty::Tuple(fields) => Some(TypeIdentity(2, fields.as_ptr() as usize)),
                Ty::Array { element, .. } => {
                    Some(TypeIdentity(3, std::sync::Arc::as_ptr(element) as usize))
                }
                _ => None,
            };
            if let Some(identity) = identity {
                self.work(active.len())?;
                if active.as_slice().contains(&identity) {
                    return Err(SourceError {
                        kind: SourceErrorKind::Contract,
                        primary: None,
                        message: "coverage domain contains a by-value type cycle".into(),
                        _diagnostic_lease: None,
                        location: None,
                    }
                    .to_string());
                }
                push(&mut active, identity)?;
                push(&mut pending, DomainEvent::Leave)?;
            }
            let next = depth.checked_add(1).ok_or_else(overflow)?;
            match ty {
                Ty::Opaque(_)
                | Ty::Closure(_)
                | Ty::FnItem(_)
                | Ty::Never
                | Ty::Slice { .. }
                | Ty::Str
                | Ty::Dyn(_)
                | Ty::AssociatedType(_) => applicable = false,
                Ty::Tuple(fields) => {
                    if fields.is_empty() {
                        applicable = false;
                    }
                    for child in fields.iter().rev() {
                        self.work(1)?;
                        push(&mut pending, DomainEvent::Enter(child, next))?;
                    }
                }
                Ty::Array { element, len } => {
                    if *len > ir::MAX_ARRAY_LEN {
                        return Err("array length limit exceeded".into());
                    }
                    if *len != 0 {
                        self.work(1)?;
                        push(&mut pending, DomainEvent::Enter(element, next))?;
                    }
                }
                Ty::Struct(value) => {
                    for field in value.def().fields.iter().rev() {
                        self.work(1)?;
                        push(&mut pending, DomainEvent::Enter(&field.ty, next))?;
                    }
                }
                Ty::Enum(value) => {
                    if value.def().variants.is_empty() {
                        applicable = false;
                    } else {
                        let mut count = value.def().variants.len();
                        for variant in value.def().variants.iter() {
                            self.work(1)?;
                            count = count
                                .checked_add(variant.fields.len())
                                .ok_or_else(overflow)?;
                        }
                        self.work(count)?;
                        value.validate()?;
                    }
                    for variant in value.def().variants.iter().rev() {
                        self.work(1)?;
                        for field in variant.fields.iter().rev() {
                            self.work(1)?;
                            push(&mut pending, DomainEvent::Enter(&field.ty, next))?;
                        }
                    }
                }
                // References stay opaque for inhabitance (`&!` is not treated
                // as empty, and referents may be recursive). Rows still test
                // through them with explicit Deref projections.
                Ty::Param(_)
                | Ty::Ref { .. }
                | Ty::RawPtr { .. }
                | Ty::Int(_)
                | Ty::Float(_)
                | Ty::Char
                | Ty::Bool
                | Ty::Unit => {}
            }
        }
        // Only fully facts-validated inhabitance boundaries may fall back.
        // Detected by-value cycles fail closed above, before recursive facts.
        self.facts
            .validate_ty_with_budget(&self.domain, &self.plan)?;
        Ok(applicable)
    }

    pub(super) fn insert_row<'row, F>(&mut self, count: usize, mut at: F) -> Result<bool, String>
    where
        F: FnMut(usize) -> TestRef<'row>,
    {
        self.work(count)?;
        if count > ir::MAX_PATTERN_BINDINGS {
            return Err("coverage row exceeds pattern test limit".into());
        }
        let mut row = Row {
            tests: temporary(&self.expansion)?,
        };
        for index in 0..count {
            self.work(1)?;
            let source = at(index);
            self.plan.check_depth(source.projection.len())?;
            self.work(source.projection.len())?;
            for earlier in row.tests.as_slice() {
                if self.same_path(earlier.projection.as_slice(), source.projection)? {
                    return Err("coverage row repeats a tested path".into());
                }
            }
            self.validate_path(source.projection, row.tests.as_slice())?;
            let actual = ir::projected_ty(&self.domain, source.projection)?;
            match source.kind {
                PatternTestKind::Bool(_) if matches!(actual, Ty::Bool) => {}
                PatternTestKind::Int(_)
                | PatternTestKind::Char(_)
                | PatternTestKind::IntRange { .. }
                | PatternTestKind::CharRange { .. }
                    if source.kind.scalar_ty().as_ref() == Some(actual) =>
                {
                    source.kind.scalar_interval()?;
                }
                PatternTestKind::EnumVariant { ty, variant } => {
                    self.work(ty.def().variants.len())?;
                    if *variant >= ty.def().variants.len()
                        || !ir::identical_type(actual, &Ty::Enum(ty.clone()), &self.plan)?
                    {
                        return Err("coverage tag type, allocation or variant mismatch".into());
                    }
                }
                _ => return Err("coverage test type mismatch".into()),
            }
            let mut projection = temporary(&self.expansion)?;
            for step in source.projection {
                self.work(1)?;
                push(&mut projection, *step)?;
            }
            push(
                &mut row.tests,
                Test {
                    projection,
                    kind: source.kind.clone(),
                },
            )?;
        }
        // No shortcut can bypass full candidate validation. Commit only after
        // successful comparison against PRIOR rows, even when this row is dead.
        let useful = self.useful(&row)?;
        self.commit_row(row)?;
        Ok(useful)
    }

    fn commit_row(&mut self, row: Row) -> Result<(), String> {
        self.work(1)?;
        if self.rows.values.len() < self.rows.values.capacity() {
            self.rows.values.push(row);
            return Ok(());
        }
        // Admission is transactional: growing the OLD vector first could lose
        // prior rows if an allocator-capacity top-up fails after reallocation.
        let capacity = self
            .rows
            .values
            .len()
            .max(2)
            .checked_mul(2)
            .ok_or_else(overflow)?;
        let mut staged = Rows {
            values: Vec::new(),
            lease: self
                .expansion
                .reserve_array(
                    Storage::Temporary,
                    u64::try_from(capacity).map_err(|_| overflow())?,
                    std::mem::size_of::<Row>() as u64,
                )
                .map_err(|error| token_error(error.into()))?,
        };
        if staged.values.try_reserve_exact(capacity).is_err() {
            return Err(token_error(TokenError::Budget(
                self.expansion.poison_allocation(),
            )));
        }
        let actual = u64::try_from(staged.values.capacity())
            .map_err(|_| overflow())?
            .checked_mul(std::mem::size_of::<Row>() as u64)
            .ok_or_else(overflow)?;
        staged
            .lease
            .resize(actual)
            .map_err(|error| token_error(error.into()))?;
        self.work(self.rows.values.len())?;
        // Capacity and every fallible operation are complete. These moves and
        // pushes cannot allocate, fail, or discard the prior rows.
        staged.values.append(&mut self.rows.values);
        staged.values.push(row);
        std::mem::swap(&mut staged, &mut self.rows);
        Ok(())
    }

    fn validate_path(&self, path: &[Projection], earlier: &[Test]) -> Result<(), String> {
        for (index, step) in path.iter().enumerate() {
            self.work(1)?;
            match step {
                Projection::DynamicIndex(_) => {
                    return Err("runtime index is not a source pattern projection".into());
                }
                Projection::Field(_) | Projection::ConstantIndex(_) | Projection::Deref => {}
                Projection::Downcast(variant) => {
                    let mut found = false;
                    for previous in earlier {
                        if self.same_path(previous.projection.as_slice(), &path[..index])?
                            && let PatternTestKind::EnumVariant {
                                ty,
                                variant: selected,
                            } = &previous.kind
                            && ty.def().id == variant.owner
                            && *selected == variant.index
                        {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Err("coverage payload path lacks an earlier exact tag".into());
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn exhaustive(&self) -> Result<bool, String> {
        self.work(0)?;
        let wildcard = Row {
            tests: temporary(&self.expansion)?,
        };
        Ok(!self.useful(&wildcard)?)
    }

    fn state<'a>(&self, row: &'a Row, trail: &[Assignment<'a>]) -> Result<State<'a>, String> {
        for test in row.tests.as_slice() {
            self.work(1)?;
            let mut value = None;
            for assigned in trail {
                if self.same_path(
                    test.projection.as_slice(),
                    assigned.test.projection.as_slice(),
                )? {
                    value = Some(assigned.value);
                    break;
                }
            }
            let Some(value) = value else {
                // Earlier tests of THIS row have already evaluated true, so
                // the first unassigned path carries its real tag prerequisites.
                return Ok(State::Open(test));
            };
            let matches = match (&test.kind, value) {
                (PatternTestKind::Bool(a), Cell::Bool(b)) => *a == b,
                (kind, Cell::Scalar(point)) if kind.scalar_ty().is_some() => {
                    let (lo, hi) = kind
                        .scalar_interval()?
                        .ok_or("coverage scalar interval invariant")?;
                    lo <= point && point <= hi
                }
                (PatternTestKind::EnumVariant { variant, .. }, Cell::Variant(b)) => *variant == b,
                _ => return Err("coverage assignment domain mismatch".into()),
            };
            if !matches {
                return Ok(State::False);
            }
        }
        Ok(State::True)
    }

    fn check_runtime_path(&self, test: &Test, trail: &[Assignment<'_>]) -> Result<(), String> {
        for (index, step) in test.projection.as_slice().iter().enumerate() {
            self.work(1)?;
            let Projection::Downcast(variant) = step else {
                continue;
            };
            let mut found = false;
            for assignment in trail {
                if self.same_path(
                    assignment.test.projection.as_slice(),
                    &test.projection.as_slice()[..index],
                )? && let PatternTestKind::EnumVariant { ty, .. } = &assignment.test.kind
                    && ty.def().id == variant.owner
                    && assignment.value == Cell::Variant(variant.index)
                {
                    found = true;
                    break;
                }
            }
            if !found {
                return Err("coverage split lacks an active exact tag refinement".into());
            }
        }
        Ok(())
    }

    fn frame<'a>(
        &'a self,
        test: &'a Test,
        query: &'a Row,
        base: usize,
    ) -> Result<Frame<'a>, String> {
        let mut integers = temporary(&self.expansion)?;
        let kind = match &test.kind {
            PatternTestKind::Bool(_) => ChoiceKind::Bool,
            PatternTestKind::EnumVariant { ty, .. } => ChoiceKind::Enum(ty.def().variants.len()),
            kind => {
                let domain = kind.scalar_ty().ok_or("coverage scalar domain invariant")?;
                let (max, gap) = match domain {
                    Ty::Int(ty) => (ty.mask(), None),
                    Ty::Char => (0x10ffff, Some((0xd800u128, 0xdfffu128))),
                    _ => return Err("coverage scalar domain invariant".into()),
                };
                let mut starts = temporary(&self.expansion)?;
                push(&mut starts, 0)?;
                if let Some((lo, hi)) = gap {
                    push(&mut starts, lo)?;
                    push(&mut starts, hi + 1)?;
                }
                for row in self.rows.as_slice().iter().chain(std::iter::once(query)) {
                    for candidate in row.tests.as_slice() {
                        if self.same_path(
                            candidate.projection.as_slice(),
                            test.projection.as_slice(),
                        )? && let Some((lo, hi)) = candidate.kind.scalar_interval()?
                        {
                            self.work(2)?;
                            push(&mut starts, lo)?;
                            if hi < max {
                                push(&mut starts, hi + 1)?;
                            }
                        }
                    }
                }
                let count = starts.len();
                self.work(
                    count
                        .checked_mul(usize::BITS as usize)
                        .ok_or_else(overflow)?,
                )?;
                starts.sort_unstable_dedup();
                for &start in starts.as_slice() {
                    // Surrogates are not scalar values: that segment is uninhabited.
                    if gap.is_some_and(|(lo, _)| lo == start) {
                        continue;
                    }
                    push(&mut integers, start)?;
                }
                ChoiceKind::Scalar
            }
        };
        Ok(Frame {
            integers,
            test,
            kind,
            next: 0,
            base,
        })
    }

    fn advance<'a>(
        &self,
        frames: &mut BudgetVec<Frame<'a>>,
        trail: &mut BudgetVec<Assignment<'a>>,
    ) -> Result<bool, String> {
        while let Some(mut frame) = frames.pop() {
            self.work(1)?;
            trail.truncate(frame.base);
            if let Some(value) = frame.next_cell() {
                push(
                    &mut *trail,
                    Assignment {
                        test: frame.test,
                        value,
                    },
                )?;
                push(&mut *frames, frame)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn useful<'a>(&'a self, query: &'a Row) -> Result<bool, String> {
        let mut frames = temporary(&self.expansion)?;
        let mut trail = temporary(&self.expansion)?;
        // A validated query is a positive conjunction with unique paths. Its
        // exact values are constraints, not branches to enumerate and reject.
        // Tags are installed in source-test order before payload constraints.
        for test in query.tests.as_slice() {
            self.work(1)?;
            self.check_runtime_path(test, trail.as_slice())?;
            let value = match &test.kind {
                PatternTestKind::Bool(value) => Cell::Bool(*value),
                PatternTestKind::EnumVariant { variant, .. } => Cell::Variant(*variant),
                // A scalar constraint may straddle other rows' segments. It
                // stays open, so the search splits on it and prunes outside cells.
                _ => continue,
            };
            push(&mut trail, Assignment { test, value })?;
        }
        loop {
            self.work(1)?;
            let query_state = self.state(query, trail.as_slice())?;
            if !matches!(query_state, State::False) {
                let mut split = match query_state {
                    State::Open(test) => Some(test),
                    _ => None,
                };
                let mut covered = false;
                for row in self.rows.as_slice() {
                    match self.state(row, trail.as_slice())? {
                        State::True => {
                            covered = true;
                            break;
                        }
                        State::Open(test) if split.is_none() => split = Some(test),
                        _ => {}
                    }
                }
                if !covered {
                    let Some(test) = split else {
                        return Ok(true);
                    };
                    self.check_runtime_path(test, trail.as_slice())?;
                    let depth = frames.len().checked_add(1).ok_or_else(overflow)?;
                    self.plan.check_depth(depth)?;
                    push(&mut frames, self.frame(test, query, trail.len())?)?;
                }
            }
            if !self.advance(&mut frames, &mut trail)? {
                return Ok(false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{
        expansion_budget::{BudgetFailureKind, ExpansionLimits},
        module_identity::ModuleArenaBuilder,
    };
    use super::*;
    use std::sync::Arc;

    fn matrix(domain: Ty) -> CoverageMatrix<'static> {
        CoverageMatrix::new(
            domain,
            FactsCtx::Concrete,
            PlanBudget::default(),
            ExpansionBudget::default(),
        )
        .unwrap()
        .unwrap()
    }
    fn insert(matrix: &mut CoverageMatrix<'_>, tests: &[ir::PatternTest]) -> Result<bool, String> {
        matrix.insert_row(tests.len(), |index| TestRef {
            projection: &tests[index].projection,
            kind: &tests[index].kind,
        })
    }
    fn boolean(field: usize, value: bool) -> ir::PatternTest {
        ir::PatternTest {
            projection: vec![Projection::Field(field)],
            kind: PatternTestKind::Bool(value),
        }
    }

    #[test]
    fn sparse_products_cover_collectively_and_dead_rows_are_retained() {
        let mut matrix = matrix(Ty::Tuple(Arc::from([Ty::Bool, Ty::Bool])));
        assert!(insert(&mut matrix, &[boolean(0, true)]).unwrap());
        assert!(insert(&mut matrix, &[boolean(0, false), boolean(1, true)]).unwrap());
        assert!(!matrix.exhaustive().unwrap());
        assert!(insert(&mut matrix, &[boolean(0, false), boolean(1, false)]).unwrap());
        assert!(matrix.exhaustive().unwrap());
        assert!(!insert(&mut matrix, &[boolean(1, false)]).unwrap());
        assert_eq!(matrix.rows.values.len(), 4);
    }

    #[test]
    fn integer_128_other_is_symbolic_and_noncanonical_bits_fail_even_after_wildcard() {
        for ty in [ir::IntTy::I128, ir::IntTy::U128] {
            let mut matrix = matrix(Ty::Int(ty));
            for bits in [0, u128::MAX, 1u128 << 127] {
                assert!(
                    insert(
                        &mut matrix,
                        &[ir::PatternTest {
                            projection: vec![],
                            kind: PatternTestKind::Int(ir::IntValue { ty, bits }),
                        }]
                    )
                    .unwrap()
                );
            }
            assert!(!matrix.exhaustive().unwrap());
            assert!(insert(&mut matrix, &[]).unwrap());
            assert!(matrix.exhaustive().unwrap());
        }
        let mut matrix = matrix(Ty::Int(ir::IntTy::U8));
        assert!(insert(&mut matrix, &[]).unwrap());
        let rows = matrix.rows.values.len();
        assert!(
            insert(
                &mut matrix,
                &[ir::PatternTest {
                    projection: vec![],
                    kind: PatternTestKind::Int(ir::IntValue {
                        ty: ir::IntTy::U8,
                        bits: 256
                    }),
                }]
            )
            .is_err()
        );
        assert_eq!(matrix.rows.values.len(), rows);
    }

    #[test]
    fn finite_integer_partition_omits_other_only_at_actual_cardinality() {
        let mut matrix = matrix(Ty::Int(ir::IntTy::U8));
        // Isolate the partition builder, without making 256 repeated full
        // usefulness queries part of this resource-bound unit control.
        for bits in 0..256 {
            let mut tests = temporary(&matrix.expansion).unwrap();
            push(
                &mut tests,
                Test {
                    projection: temporary(&matrix.expansion).unwrap(),
                    kind: PatternTestKind::Int(ir::IntValue {
                        ty: ir::IntTy::U8,
                        bits,
                    }),
                },
            )
            .unwrap();
            matrix.commit_row(Row { tests }).unwrap();
        }
        let query = Row {
            tests: temporary(&matrix.expansion).unwrap(),
        };
        let selector = &matrix.rows.as_slice()[0].tests.as_slice()[0];
        let mut frame = matrix.frame(selector, &query, 0).unwrap();
        assert!(matches!(frame.kind, ChoiceKind::Scalar));
        for bits in 0..256 {
            assert!(frame.next_cell() == Some(Cell::Scalar(bits)));
        }
        assert!(frame.next_cell().is_none());
    }

    fn scalar(kind: PatternTestKind) -> ir::PatternTest {
        ir::PatternTest {
            projection: vec![],
            kind,
        }
    }
    fn chars(lo: u32, hi: u32) -> PatternTestKind {
        PatternTestKind::CharRange {
            lo: ir::CharValue { value: lo },
            hi: ir::CharValue { value: hi },
        }
    }
    fn ints(ty: ir::IntTy, lo: u128, hi: u128) -> PatternTestKind {
        PatternTestKind::IntRange {
            lo: ir::IntValue { ty, bits: lo },
            hi: ir::IntValue { ty, bits: hi },
        }
    }

    #[test]
    fn char_intervals_skip_the_surrogate_gap_and_detect_one_missing_scalar() {
        let mut full = matrix(Ty::Char);
        assert!(insert(&mut full, &[scalar(chars(0, 0xd7ff))]).unwrap());
        assert!(!full.exhaustive().unwrap());
        assert!(insert(&mut full, &[scalar(chars(0xe000, 0x10ffff))]).unwrap());
        assert!(full.exhaustive().unwrap());
        assert!(
            !insert(
                &mut full,
                &[scalar(PatternTestKind::Char(ir::CharValue { value: 97 }))]
            )
            .unwrap()
        );
        let mut short = matrix(Ty::Char);
        insert(&mut short, &[scalar(chars(0, 0xd7ff))]).unwrap();
        insert(&mut short, &[scalar(chars(0xe000, 0x10fffe))]).unwrap();
        assert!(!short.exhaustive().unwrap());
        assert!(
            insert(
                &mut short,
                &[scalar(PatternTestKind::Char(ir::CharValue {
                    value: 0x10ffff
                }))]
            )
            .unwrap()
        );
        assert!(short.exhaustive().unwrap());
    }

    #[test]
    fn signed_and_wide_integer_intervals_use_value_order() {
        // i8: -128..=-1 is bits 0x80..=0xff, 0..=127 is 0x00..=0x7f.
        let mut signed = matrix(Ty::Int(ir::IntTy::I8));
        assert!(insert(&mut signed, &[scalar(ints(ir::IntTy::I8, 0x80, 0xff))]).unwrap());
        assert!(!signed.exhaustive().unwrap());
        assert!(insert(&mut signed, &[scalar(ints(ir::IntTy::I8, 0, 0x7f))]).unwrap());
        assert!(signed.exhaustive().unwrap());
        // -2..=1 straddles zero in value order and is dead after both rows.
        assert!(!insert(&mut signed, &[scalar(ints(ir::IntTy::I8, 0xfe, 0x01))]).unwrap());
        let mut wide = matrix(Ty::Int(ir::IntTy::U128));
        assert!(insert(&mut wide, &[scalar(ints(ir::IntTy::U128, 0, 5))]).unwrap());
        assert!(!insert(&mut wide, &[scalar(ints(ir::IntTy::U128, 2, 3))]).unwrap());
        assert!(
            insert(
                &mut wide,
                &[scalar(ints(ir::IntTy::U128, 3, u128::MAX - 1))]
            )
            .unwrap()
        );
        assert!(!wide.exhaustive().unwrap());
        assert!(
            insert(
                &mut wide,
                &[scalar(ints(ir::IntTy::U128, u128::MAX, u128::MAX))]
            )
            .unwrap()
        );
        assert!(wide.exhaustive().unwrap());
    }

    #[test]
    fn reversed_interval_rows_are_rejected_before_commit() {
        let mut matrix = matrix(Ty::Int(ir::IntTy::U8));
        assert!(insert(&mut matrix, &[scalar(ints(ir::IntTy::U8, 9, 1))]).is_err());
        assert!(insert(&mut matrix, &[scalar(ints(ir::IntTy::I8, 0, 1))]).is_err());
        assert!(insert(&mut matrix, &[scalar(chars(0, 1))]).is_err());
        assert!(matrix.rows.values.is_empty());
    }

    #[test]
    fn all_u8_singleton_rows_are_exhaustive_under_unchanged_default_budgets() {
        let mut matrix = matrix(Ty::Int(ir::IntTy::U8));
        for bits in 0..256 {
            assert!(
                insert(
                    &mut matrix,
                    &[ir::PatternTest {
                        projection: vec![],
                        kind: PatternTestKind::Int(ir::IntValue {
                            ty: ir::IntTy::U8,
                            bits
                        }),
                    }]
                )
                .unwrap()
            );
        }
        assert!(matrix.exhaustive().unwrap());
        assert!(matrix.plan.first_resource_failure().is_none());
        assert!(matrix.expansion.snapshot().failure.is_none());
    }

    fn payload_enum() -> (
        ir::EnumTy,
        Arc<super::super::super::module_identity::ModuleArena>,
    ) {
        let modules = ModuleArenaBuilder::for_catalog(ExpansionBudget::default())
            .unwrap()
            .freeze()
            .unwrap();
        let snapshot = ir::SnapshotId::allocate().unwrap();
        let source_decl = super::super::super::traits::test_support::source_decl(
            snapshot,
            0,
            &PlanBudget::default(),
        )
        .unwrap();
        let value = ir::EnumTy::new(ir::EnumDef {
            source_decl,
            type_args: Arc::from([]),
            id: ir::NominalAppId {
                snapshot,
                index: crate::compiler::ir::NominalIndex::SourceSlot(0),
            },
            name: "TwoPayloads".into(),
            module: modules.core_root(),
            visibility: ir::Visibility::Public,
            variants: ["A", "B"]
                .into_iter()
                .map(|name| ir::VariantDef {
                    name: name.into(),
                    shape: ir::VariantShape::Tuple,
                    fields: Arc::from([ir::FieldDef {
                        name: "0".into(),
                        ty: Ty::Bool,
                        visibility: ir::Visibility::Public,
                        lifetime_bound: false,
                    }]),
                })
                .collect::<Vec<_>>()
                .into(),
        });
        (value, modules)
    }

    #[test]
    fn enum_partition_cell_not_anchor_row_tag_authorizes_payload() {
        let (value, _modules) = payload_enum();
        let mut matrix = matrix(Ty::Enum(value.clone()));
        for variant in 0..2 {
            for scalar in [true, false] {
                let tag = ir::PatternTest {
                    projection: vec![],
                    kind: PatternTestKind::EnumVariant {
                        ty: value.clone(),
                        variant,
                    },
                };
                let payload = ir::PatternTest {
                    projection: vec![
                        Projection::Downcast(ir::VariantId {
                            owner: value.def().id,
                            index: variant,
                        }),
                        Projection::Field(0),
                    ],
                    kind: PatternTestKind::Bool(scalar),
                };
                assert!(insert(&mut matrix, &[tag, payload]).unwrap());
            }
        }
        assert!(matrix.exhaustive().unwrap());
        let count = matrix.rows.values.len();
        // Covered/dead rows cannot bypass their own tag prerequisites.
        assert!(
            insert(
                &mut matrix,
                &[ir::PatternTest {
                    projection: vec![
                        Projection::Downcast(ir::VariantId {
                            owner: value.def().id,
                            index: 0,
                        }),
                        Projection::Field(0)
                    ],
                    kind: PatternTestKind::Bool(true),
                }]
            )
            .is_err()
        );
        assert_eq!(matrix.rows.values.len(), count);
    }

    #[test]
    fn explicit_inhabitance_boundary_does_not_become_resource_fallback() {
        for domain in [
            Ty::Never,
            Ty::Tuple(Arc::from([Ty::Bool, Ty::Never])),
            Ty::Array {
                element: Arc::new(Ty::Never),
                len: 1,
            },
        ] {
            assert!(
                CoverageMatrix::new(
                    domain,
                    FactsCtx::Concrete,
                    PlanBudget::default(),
                    ExpansionBudget::default()
                )
                .unwrap()
                .is_none()
            );
        }
        assert!(
            CoverageMatrix::new(
                Ty::Array {
                    element: Arc::new(Ty::Never),
                    len: 0
                },
                FactsCtx::Concrete,
                PlanBudget::default(),
                ExpansionBudget::default()
            )
            .unwrap()
            .is_some()
        );
        let plan = PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..ir::PlanLimits::default()
        });
        assert!(
            CoverageMatrix::new(
                Ty::Never,
                FactsCtx::Concrete,
                plan.clone(),
                ExpansionBudget::default()
            )
            .is_err()
        );
        assert_eq!(
            plan.first_resource_failure().unwrap().kind,
            ir::ResourceBudgetKind::Work
        );
    }

    #[test]
    fn growth_denial_keeps_prior_rows_and_drop_releases_after_poison() {
        let expansion = ExpansionBudget::new(ExpansionLimits {
            bytes: (std::mem::size_of::<Row>() * 8) as u64,
            ..ExpansionLimits::default()
        });
        let mut matrix = CoverageMatrix::new(
            Ty::Bool,
            FactsCtx::Concrete,
            PlanBudget::default(),
            expansion.clone(),
        )
        .unwrap()
        .unwrap();
        for _ in 0..4 {
            insert(&mut matrix, &[]).unwrap();
        }
        assert_eq!(matrix.rows.values.len(), 4);
        assert!(insert(&mut matrix, &[]).is_err());
        assert_eq!(matrix.rows.values.len(), 4);
        assert_eq!(
            expansion.snapshot().failure.unwrap().kind,
            BudgetFailureKind::Limit
        );
        drop(matrix);
        assert_eq!(expansion.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn work_failure_is_latched_before_callback_and_preserves_rows() {
        let plan = PlanBudget::new(ir::PlanLimits {
            work: 128,
            ..ir::PlanLimits::default()
        });
        let expansion = ExpansionBudget::default();
        let mut matrix = CoverageMatrix::new(
            Ty::Bool,
            FactsCtx::Concrete,
            plan.clone(),
            expansion.clone(),
        )
        .unwrap()
        .unwrap();
        insert(&mut matrix, &[]).unwrap();
        let mut called = false;
        let atom = PatternTestKind::Bool(true);
        assert!(
            matrix
                .insert_row(129, |_| {
                    called = true;
                    TestRef {
                        projection: &[],
                        kind: &atom,
                    }
                })
                .is_err()
        );
        assert!(!called);
        assert_eq!(matrix.rows.values.len(), 1);
        let failure = plan.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Work);
        assert!(matrix.exhaustive().is_err());
        assert_eq!(plan.first_resource_failure(), Some(failure));
        drop(matrix);
        assert_eq!(expansion.snapshot().temporary_bytes, 0);
    }
}
