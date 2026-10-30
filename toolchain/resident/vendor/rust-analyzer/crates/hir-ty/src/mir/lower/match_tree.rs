//! Which arms of a `match` rustc builds a way into.
//!
//! rustc lowers the arms of a `match` to one tree of tests (`rustc_mir_build`'s
//! `builder::matches`). An arm that an arm before it has taken every value from is still
//! built, but no edge of the tree leads to it, and its analysis of what a drop finds moved out
//! never sees what that arm binds. rustd tests the arms one after the other, where every arm
//! is on some way. This follows rustc's construction far enough to say which arms it gets
//! to, and builds nothing: the candidates are sorted by the same tests in the same order.

use std::rc::Rc;

use hir_def::{
    EnumVariantId, Lookup, VariantId,
    hir::{Expr, Literal, MatchArm, Pat, PatId, UnaryOp},
};
use rustc_type_ir::inherent::IntoKind;
use syntax::ast::RangeOp;

use crate::{
    infer::PatAdjust,
    mir::lower::{MirLowerCtx, Ty, TyKind},
};

/// One step from a matched value to a part of it.
#[derive(Clone, PartialEq)]
enum Step {
    Field(u32),
    Variant(EnumVariantId),
    Index(u64),
    FromEnd(u64),
    Deref,
}

/// What a pattern asks of the place it is at.
#[derive(Clone, PartialEq)]
enum Ask {
    Variant(EnumVariantId),
    /// An integer or a `char`, as a number that orders the way the type does.
    Int(u128),
    Bool(bool),
    /// Both ends are in it.
    Range(u128, u128),
    /// A constant that is compared with `==`.
    Equal(Literal),
    Len {
        len: u64,
        or_more: bool,
    },
    Or(Vec<Vec<Pair>>),
}

/// A place, what a pattern asks of it, and what the pattern asks inside once that holds.
#[derive(Clone, PartialEq)]
struct Pair {
    place: Vec<Step>,
    ask: Ask,
    inside: Vec<Pair>,
}

/// One way an arm can match: what is still to be tested for it.
#[derive(Clone)]
struct Candidate {
    arm: usize,
    pairs: Vec<Pair>,
    guarded: bool,
    /// What an or-pattern left to be tested after its alternative has matched.
    then: Option<Rc<[Pair]>>,
}

/// Where a test sends a candidate.
#[derive(Clone, PartialEq)]
enum Branch {
    Variant(EnumVariantId),
    Int(u128),
    Success,
    Failure,
}

/// The run over the candidates. A `bool` that goes with a list of candidates says whether
/// the block they start from can be reached.
struct Tree {
    reached: Vec<bool>,
    /// Whether what an or-pattern left to be tested has failed since the or-pattern was
    /// expanded: that goes to where the whole expansion falls through to.
    escaped: bool,
}

impl Tree {
    /// Matches the candidates in order, and says whether none of them matching can happen.
    /// Or-patterns are expanded before any test, as soon as any candidate starts with one,
    /// so that the alternatives are sorted by the tests of the arms before them.
    fn run(&mut self, mut start: bool, mut candidates: Vec<Candidate>) -> bool {
        loop {
            let Some(first) = candidates.first() else { return start };
            match first.pairs.first() {
                None => start = self.matched(start, candidates.remove(0)),
                Some(_) if candidates.iter().any(starts_with_or) => {
                    (start, candidates) = self.expand(start, candidates)
                }
                Some(_) => (start, candidates) = self.test(start, candidates),
            }
        }
    }

    /// A candidate with nothing left to test. Says whether the candidates after it can
    /// still be reached: only through a guard.
    fn matched(&mut self, start: bool, candidate: Candidate) -> bool {
        let Candidate { arm, guarded, then, .. } = candidate;
        let Some(then) = then else {
            self.reached[arm] |= start;
            return start && guarded;
        };
        let rest = Candidate { arm, pairs: then.to_vec(), guarded, then: None };
        let fell = self.run(start, vec![rest]);
        if guarded {
            return fell;
        }
        self.escaped |= fell;
        false
    }

    /// Expands the or-patterns that candidates start with into a candidate for each
    /// alternative. The candidates after one that has more to test after its or-pattern are
    /// left for after.
    fn expand(&mut self, start: bool, mut candidates: Vec<Candidate>) -> (bool, Vec<Candidate>) {
        let last = candidates.iter().position(|it| starts_with_or(it) && it.pairs.len() > 1);
        let remaining = candidates.split_off(last.map_or(candidates.len(), |it| it + 1));
        let mut expanded = Vec::new();
        for mut candidate in candidates {
            if !starts_with_or(&candidate) {
                expanded.push(candidate);
                continue;
            }
            let Ask::Or(alternatives) = candidate.pairs.remove(0).ask else { continue };
            let after = candidate.then.iter().flat_map(|it| it.iter().cloned());
            let left: Vec<Pair> = candidate.pairs.into_iter().chain(after).collect();
            let then: Option<Rc<[Pair]>> = (!left.is_empty()).then(|| left.into());
            expanded.extend(alternatives.into_iter().map(|pairs| Candidate {
                arm: candidate.arm,
                pairs,
                guarded: candidate.guarded,
                then: then.clone(),
            }));
        }
        let outer = std::mem::take(&mut self.escaped);
        let fell = self.run(start, expanded);
        let escaped = std::mem::replace(&mut self.escaped, outer);
        (fell || escaped, remaining)
    }

    /// Makes the test the first candidate asks for first, sorts the candidates that the
    /// outcome says something about into its branches, and matches each branch. The
    /// candidates from the first one it says nothing about are left for after.
    fn test(&mut self, start: bool, candidates: Vec<Candidate>) -> (bool, Vec<Candidate>) {
        let Pair { place, ask, .. } = candidates[0].pairs[0].clone();
        let mut branches: Vec<(Branch, Vec<Candidate>)> = Vec::new();
        let mut remaining = Vec::new();
        let mut candidates = candidates.into_iter();
        for mut candidate in candidates.by_ref() {
            let Some(branch) = sort(&place, &ask, &mut candidate, &branches) else {
                remaining.push(candidate);
                break;
            };
            match branches.iter_mut().find(|(it, _)| *it == branch) {
                Some((_, sorted)) => sorted.push(candidate),
                None => branches.push((branch, vec![candidate])),
            }
        }
        remaining.extend(candidates);
        // A switch always has a target for the values none of the candidates named, and
        // the test of a `bool` has one for the value the first candidate did not ask for.
        let has = |branch: Branch| branches.iter().any(|(it, _)| *it == branch);
        let mut fell = start
            && match ask {
                Ask::Bool(_) => !(has(Branch::Success) && has(Branch::Failure)),
                _ => !has(Branch::Failure),
            };
        for (_, sorted) in branches {
            fell |= self.run(start, sorted);
        }
        (fell, remaining)
    }
}

/// Whether the first thing left to test for `candidate` is an or-pattern.
fn starts_with_or(candidate: &Candidate) -> bool {
    matches!(candidate.pairs.first(), Some(Pair { ask: Ask::Or(_), .. }))
}

/// The branch of the test `ask` makes of `place` that `candidate` belongs in, if the outcome
/// decides anything for it. A candidate the test answers a pair of loses the pair and gets
/// what was inside it, with the or-patterns last.
fn sort(
    place: &[Step],
    ask: &Ask,
    candidate: &mut Candidate,
    branches: &[(Branch, Vec<Candidate>)],
) -> Option<Branch> {
    let at = candidate.pairs.iter().position(|it| it.place == place)?;
    let (branch, answered) = match (ask, &candidate.pairs[at].ask) {
        (Ask::Variant(_), &Ask::Variant(variant)) => (Branch::Variant(variant), true),
        (Ask::Int(_), &Ask::Int(value)) => {
            let covers = |it: &Pair| {
                it.place == place
                    && matches!(it.ask, Ask::Range(lo, hi) if lo <= value && value <= hi)
            };
            let failed = branches.iter().filter(|(it, _)| *it == Branch::Failure);
            if failed.flat_map(|(_, sorted)| sorted).any(|it| it.pairs.iter().any(covers)) {
                return None;
            }
            (Branch::Int(value), true)
        }
        (Ask::Int(_), &Ask::Range(lo, hi)) => {
            let named = |it: &(Branch, Vec<Candidate>)| matches!(it.0, Branch::Int(value) if lo <= value && value <= hi);
            if branches.iter().any(named) {
                return None;
            }
            (Branch::Failure, false)
        }
        (Ask::Bool(_), &Ask::Bool(value)) => {
            (if value { Branch::Success } else { Branch::Failure }, true)
        }
        (Ask::Range(lo, hi), &Ask::Range(other_lo, other_hi)) => {
            if (lo, hi) == (&other_lo, &other_hi) {
                (Branch::Success, true)
            } else if other_hi < *lo || *hi < other_lo {
                (Branch::Failure, false)
            } else {
                return None;
            }
        }
        (Ask::Range(lo, hi), &Ask::Int(value)) if value < *lo || *hi < value => {
            (Branch::Failure, false)
        }
        (Ask::Equal(value), Ask::Equal(other)) if value == other => (Branch::Success, true),
        (&Ask::Len { len: tested, or_more: false }, &Ask::Len { len, or_more }) => {
            match (tested.cmp(&len), or_more) {
                (std::cmp::Ordering::Equal, false) => (Branch::Success, true),
                (std::cmp::Ordering::Less, _) | (std::cmp::Ordering::Greater, false) => {
                    (Branch::Failure, false)
                }
                _ => return None,
            }
        }
        (&Ask::Len { len: tested, or_more: true }, &Ask::Len { len, or_more }) => {
            match (tested.cmp(&len), or_more) {
                (std::cmp::Ordering::Equal, true) => (Branch::Success, true),
                (std::cmp::Ordering::Greater, true) => return None,
                _ => (Branch::Failure, false),
            }
        }
        _ => return None,
    };
    if answered {
        let pair = candidate.pairs.remove(at);
        candidate.pairs.extend(pair.inside);
        candidate.pairs.sort_by_key(|it| matches!(it.ask, Ask::Or(_)));
    }
    Some(branch)
}

impl<'db> MirLowerCtx<'_, 'db> {
    /// For each arm, whether rustc's tree of tests has a way into it. Every arm, where a
    /// pattern has something in it that is not followed here.
    pub(super) fn reached_arms(&self, arms: &[MatchArm]) -> Vec<bool> {
        let candidates = arms.iter().enumerate().map(|(arm, it)| {
            let mut pairs = Vec::new();
            self.pairs_of(it.pat, Vec::new(), &mut pairs)?;
            pairs.sort_by_key(|it| matches!(it.ask, Ask::Or(_)));
            Some(Candidate { arm, pairs, guarded: it.guard.is_some(), then: None })
        });
        let Some(candidates) = candidates.collect::<Option<Vec<_>>>() else {
            return vec![true; arms.len()];
        };
        let mut tree = Tree { reached: vec![false; arms.len()], escaped: false };
        tree.run(true, candidates);
        tree.reached
    }

    /// Adds what `pattern` asks of the value at `place` to `pairs`. `None` when the pattern
    /// has a part whose test is not known here.
    fn pairs_of(&self, pattern: PatId, mut place: Vec<Step>, pairs: &mut Vec<Pair>) -> Option<()> {
        // A deref through a smart pointer that is not a `Box` is a call into a temporary, a
        // test of its own that is not followed here.
        let adjustments = self.infer.pat_adjustments.get(&pattern).map_or(&[][..], |it| &**it);
        for adjustment in adjustments {
            if adjustment.kind == PatAdjust::OverloadedDeref && !adjustment.source.as_ref().is_box()
            {
                return None;
            }
            place.push(Step::Deref);
        }
        let ty = self.infer.pat_ty(pattern);
        match &self.store[pattern] {
            Pat::Wild | Pat::Bind { subpat: None, .. } => (),
            &Pat::Bind { subpat: Some(inner), .. } => self.pairs_of(inner, place, pairs)?,
            &Pat::Ref { pat: inner, .. } | &Pat::Box { inner } => {
                place.push(Step::Deref);
                self.pairs_of(inner, place, pairs)?
            }
            &Pat::Deref { inner } if ty.is_box() => {
                place.push(Step::Deref);
                self.pairs_of(inner, place, pairs)?
            }
            Pat::Tuple { args, ellipsis } => {
                let TyKind::Tuple(fields) = ty.kind() else { return None };
                self.pairs_of_fields(args, *ellipsis, fields.len(), &place, pairs)?
            }
            Pat::Or(alternatives) => {
                let alternatives = alternatives.iter().map(|&it| {
                    let mut pairs = Vec::new();
                    self.pairs_of(it, place.clone(), &mut pairs)?;
                    pairs.sort_by_key(|it| matches!(it.ask, Ask::Or(_)));
                    Some(pairs)
                });
                let ask = Ask::Or(alternatives.collect::<Option<_>>()?);
                pairs.push(Pair { place, ask, inside: Vec::new() })
            }
            Pat::Lit(literal) => {
                let ask = self.constant_ask(ty, *literal)?;
                pairs.push(Pair { place, ask, inside: Vec::new() })
            }
            Pat::Range { start, end, range_type } => {
                let (least, most, bounded) = int_bounds(ty)?;
                let end_of = |end: &Option<_>, missing| match end {
                    Some(it) => match self.constant_ask(ty, *it)? {
                        Ask::Int(value) => Some(value),
                        _ => None,
                    },
                    None => Some(missing),
                };
                let (lo, hi) = (end_of(start, least)?, end_of(end, most)?);
                let hi = match (range_type, end) {
                    (RangeOp::Exclusive, Some(_)) => hi.checked_sub(1)?,
                    _ => hi,
                };
                let ask = match lo.cmp(&hi) {
                    std::cmp::Ordering::Greater => return None,
                    std::cmp::Ordering::Equal => Ask::Int(lo),
                    // A range of every value of the type asks nothing.
                    std::cmp::Ordering::Less if bounded && (lo, hi) == (least, most) => {
                        return Some(());
                    }
                    std::cmp::Ordering::Less => Ask::Range(lo, hi),
                };
                pairs.push(Pair { place, ask, inside: Vec::new() })
            }
            Pat::Slice { prefix, slice, suffix } => {
                if !slice.is_none_or(|it| self.is_rest(it)) {
                    return None;
                }
                let mut inside = Vec::new();
                for (i, &element) in prefix.iter().enumerate() {
                    let mut place = place.clone();
                    place.push(Step::Index(i as u64));
                    self.pairs_of(element, place, &mut inside)?;
                }
                let named = (prefix.len() + suffix.len()) as u64;
                let len = match ty.kind() {
                    TyKind::Array(_, len) => {
                        Some(u64::try_from(crate::consteval::try_const_usize(self.db, len)?).ok()?)
                    }
                    TyKind::Slice(_) => None,
                    _ => return None,
                };
                for (i, &element) in suffix.iter().enumerate() {
                    let mut place = place.clone();
                    place.push(match len {
                        Some(len) => Step::Index(len.checked_sub(suffix.len() as u64)? + i as u64),
                        None => Step::FromEnd((suffix.len() - 1 - i) as u64),
                    });
                    self.pairs_of(element, place, &mut inside)?;
                }
                // An array has its length, and `[..]` is any slice.
                if len.is_some() || (named == 0 && slice.is_some()) {
                    pairs.extend(inside);
                } else {
                    let ask = Ask::Len { len: named, or_more: slice.is_some() };
                    pairs.push(Pair { place, ask, inside });
                }
            }
            Pat::Path(_) => {
                let variant = self.infer.variant_resolution_for_pat(pattern)?;
                self.pairs_of_variant(variant, Vec::new(), place, pairs)
            }
            Pat::TupleStruct { args, ellipsis, .. } => {
                let variant = self.infer.variant_resolution_for_pat(pattern)?;
                let fields = match variant {
                    VariantId::EnumVariantId(it) => it.fields(self.db),
                    VariantId::StructId(it) => it.fields(self.db),
                    VariantId::UnionId(_) => return None,
                };
                let mut inside = Vec::new();
                let within = variant_place(variant, &place);
                self.pairs_of_fields(args, *ellipsis, fields.fields().len(), &within, &mut inside)?;
                self.pairs_of_variant(variant, inside, place, pairs)
            }
            Pat::Record { args, .. } => {
                let variant = self.infer.variant_resolution_for_pat(pattern)?;
                let fields = match variant {
                    VariantId::EnumVariantId(it) => it.fields(self.db),
                    VariantId::StructId(it) => it.fields(self.db),
                    VariantId::UnionId(_) => return None,
                };
                let mut inside = Vec::new();
                for field in args.iter() {
                    let mut within = variant_place(variant, &place);
                    within.push(Step::Field(u32::from(fields.field(&field.name)?.into_raw())));
                    self.pairs_of(field.pat, within, &mut inside)?;
                }
                self.pairs_of_variant(variant, inside, place, pairs)
            }
            Pat::Missing | Pat::Rest | Pat::NotNull | Pat::Deref { .. } | Pat::Expr(_) => {
                return None;
            }
        }
        Some(())
    }

    /// Whether the pattern for the middle of a slice asks nothing of it: `..` or `name @ ..`.
    fn is_rest(&self, pattern: PatId) -> bool {
        match self.store[pattern] {
            Pat::Wild | Pat::Rest | Pat::Bind { subpat: None, .. } => true,
            Pat::Bind { subpat: Some(inner), .. } => self.is_rest(inner),
            _ => false,
        }
    }

    /// The patterns of the fields of a tuple or a tuple struct of `arity` fields, where
    /// `ellipsis` says how many of them come before a `..`.
    fn pairs_of_fields(
        &self,
        args: &[PatId],
        ellipsis: Option<u32>,
        arity: usize,
        place: &[Step],
        pairs: &mut Vec<Pair>,
    ) -> Option<()> {
        let before = ellipsis.map_or(args.len(), |it| it as usize);
        for (i, &arg) in args.iter().enumerate() {
            let field = if i < before { i } else { arity.checked_sub(args.len() - i)? };
            let mut place = place.to_vec();
            place.push(Step::Field(field as u32));
            self.pairs_of(arg, place, pairs)?;
        }
        Some(())
    }

    /// A pattern that names a variant: the variant is asked for when the type has another,
    /// and what the fields ask comes after that.
    fn pairs_of_variant(
        &self,
        variant: VariantId,
        inside: Vec<Pair>,
        place: Vec<Step>,
        pairs: &mut Vec<Pair>,
    ) {
        match variant {
            VariantId::EnumVariantId(it)
                if it.lookup(self.db).parent.enum_variants(self.db).variants.len() > 1 =>
            {
                pairs.push(Pair { place, ask: Ask::Variant(it), inside })
            }
            _ => pairs.extend(inside),
        }
    }

    /// What a literal in a pattern of type `ty` asks.
    fn constant_ask(&self, ty: Ty<'db>, literal: hir_def::hir::ExprId) -> Option<Ask> {
        let literal = match &self.store[literal] {
            Expr::Literal(it) => it.clone(),
            Expr::UnaryOp { expr, op: UnaryOp::Neg } => match &self.store[*expr] {
                Expr::Literal(it) => it.clone().negate()?,
                _ => return None,
            },
            _ => return None,
        };
        let signed = matches!(ty.kind(), TyKind::Int(_));
        let ordered = |value: u128| if signed { value ^ (1 << 127) } else { value };
        Some(match literal {
            Literal::Bool(it) => Ask::Bool(it),
            Literal::Char(it) => Ask::Int(u128::from(it)),
            Literal::Int(it, _) => Ask::Int(ordered(it as u128)),
            Literal::Uint(it, _) => Ask::Int(ordered(it)),
            it => Ask::Equal(it),
        })
    }
}

/// Where the fields of a variant are, from the place of the value.
fn variant_place(variant: VariantId, place: &[Step]) -> Vec<Step> {
    let mut place = place.to_vec();
    if let VariantId::EnumVariantId(it) = variant {
        place.push(Step::Variant(it));
    }
    place
}

/// The least and the greatest value of an integer type or `char`, as `Ask::Int` has them,
/// and whether a range of all of them is known to be one: not for `usize` and `isize`.
fn int_bounds(ty: Ty<'_>) -> Option<(u128, u128, bool)> {
    use rustc_type_ir::{IntTy, UintTy};
    let unsigned = |bits: u32| (0, u128::MAX >> (128 - bits));
    let signed = |bits: u32| {
        let half = 1u128 << (bits - 1);
        ((1 << 127) - half, (1 << 127) + (half - 1))
    };
    let ((least, most), bounded) = match ty.kind() {
        TyKind::Char => ((0, u128::from(char::MAX)), true),
        TyKind::Uint(UintTy::Usize) => (unsigned(64), false),
        TyKind::Int(IntTy::Isize) => (signed(64), false),
        TyKind::Uint(it) => (unsigned(it.bit_width()? as u32), true),
        TyKind::Int(it) => (signed(it.bit_width()? as u32), true),
        _ => return None,
    };
    Some((least, most, bounded))
}
