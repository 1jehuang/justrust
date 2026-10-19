//! Isolated Linux x86-64 object-layout calculator, not registered in the graph.
//!
//! This is NOT source-type admission, a Rust intrinsic, a storage plan, or a
//! calling convention. The private grammar below describes only closed scalar,
//! unit, thin-pointer and fixed-array shapes. A future source consumer must
//! authenticate its actual typed query independently. Nominal identities and
//! compiler `Ty` values never enter this module.
//!
//! Encoding v1 is `[version, node...]`. A node is `[tag, payload_words,
//! payload...]`: scalar=0 (one Scalar code), unit=1 (empty), raw-const=2,
//! raw-mut=3, shared-ref=4, mutable-ref=5 (one child node), array=6 (u64 length,
//! one child node). These are grammar tags, NOT compiler graph kind IDs.
//! Every length must be exact. Decode checks the entire pointer suffix, but
//! calculating a pointer's layout never visits or calculates its pointee.
//!
//! Input-sized buffers use only existing PlanBudget admission. Requested bytes
//! are prepaid; allocator surplus is reconciled before fill/publication. This
//! does not prepay an allocator-rounded transient peak. Scratch leases outlive
//! their buffers. Output retains immutable accounting receipts only.
//! Receipts prove accounting authority, never source or semantic authority.

use super::ir::{PlanBudget, PlanReservation, RetainedStorage};
use std::mem::size_of;

pub const GRAMMAR_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    Resource,
    Allocation,
    StorageOverflow,
    InvalidEncoding,
    UnsupportedGrammarVersion,
    UnsupportedTarget,
    UnsupportedTargetVersion,
    WrongAuthority,
    WrongType,
    WrongTarget,
    InvalidLayout,
    SizeOverflow,
    ObjectTooLarge,
}

type Result<T> = std::result::Result<T, LayoutError>;

fn ready(budget: &PlanBudget) -> Result<()> {
    let usage = budget.usage_snapshot();
    if usage.poisoned || usage.first_failure.is_some() {
        Err(LayoutError::Resource)
    } else {
        Ok(())
    }
}

fn work(budget: &PlanBudget, amount: usize) -> Result<()> {
    ready(budget)?;
    budget
        .charge_work(amount)
        .map_err(|_| LayoutError::Resource)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Architecture {
    X86_64,
    Aarch64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatingSystem {
    Linux,
    Windows,
}

/// Frozen little-endian Linux x86-64 object ABI, not host-derived data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetContract {
    pub architecture: Architecture,
    pub operating_system: OperatingSystem,
    pub version: u32,
}

impl TargetContract {
    pub const LINUX_X86_64_V1: Self = Self {
        architecture: Architecture::X86_64,
        operating_system: OperatingSystem::Linux,
        version: 1,
    };

    fn validate(self) -> Result<()> {
        if self.architecture != Architecture::X86_64
            || self.operating_system != OperatingSystem::Linux
        {
            return Err(LayoutError::UnsupportedTarget);
        }
        if self.version != 1 {
            return Err(LayoutError::UnsupportedTargetVersion);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Scalar {
    I8 = 0,
    U8 = 1,
    I16 = 2,
    U16 = 3,
    I32 = 4,
    U32 = 5,
    I64 = 6,
    U64 = 7,
    Isize = 8,
    Usize = 9,
    I128 = 10,
    U128 = 11,
    F32 = 12,
    F64 = 13,
    Bool = 14,
    Char = 15,
}

impl Scalar {
    fn decode(code: u64) -> Result<Self> {
        match code {
            0 => Ok(Self::I8),
            1 => Ok(Self::U8),
            2 => Ok(Self::I16),
            3 => Ok(Self::U16),
            4 => Ok(Self::I32),
            5 => Ok(Self::U32),
            6 => Ok(Self::I64),
            7 => Ok(Self::U64),
            8 => Ok(Self::Isize),
            9 => Ok(Self::Usize),
            10 => Ok(Self::I128),
            11 => Ok(Self::U128),
            12 => Ok(Self::F32),
            13 => Ok(Self::F64),
            14 => Ok(Self::Bool),
            15 => Ok(Self::Char),
            _ => Err(LayoutError::InvalidEncoding),
        }
    }

    fn shape(self) -> Shape {
        let size = match self {
            Self::I8 | Self::U8 | Self::Bool => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 | Self::Char => 4,
            Self::I64 | Self::U64 | Self::Isize | Self::Usize | Self::F64 => 8,
            Self::I128 | Self::U128 => 16,
        };
        Shape {
            size,
            alignment: size,
            stride: size,
            representation: Representation::Scalar(self),
            validity: match self {
                Self::Bool => Validity::Bool,
                Self::Char => Validity::UnicodeScalar,
                _ => Validity::AllBits,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    RawConst,
    RawMut,
    SharedReference,
    MutableReference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Representation {
    Scalar(Scalar),
    Unit,
    ThinPointer(PointerKind),
    Array { length: u64 },
}

/// A validity requirement, not evidence that storage has been initialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    AllBits,
    Bool,
    UnicodeScalar,
    Unit,
    RawPointer,
    Reference,
    Elements,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub size: u64,
    pub alignment: u64,
    /// Distance between successive values of this type, including zero.
    pub stride: u64,
    pub representation: Representation,
    pub validity: Validity,
}

/// Task-local only. Its buffer is destroyed before its leases release bytes.
#[derive(Debug)]
struct Staged<T> {
    buffer: Vec<T>,
    base: Option<PlanReservation>,
    surplus: Option<PlanReservation>,
}

fn storage_bytes<T>(header: usize, capacity: usize) -> Result<usize> {
    capacity
        .checked_mul(size_of::<T>())
        .and_then(|bytes| header.checked_add(bytes))
        .ok_or(LayoutError::StorageOverflow)
}

impl<T> Staged<T> {
    fn new(count: usize, header: usize, budget: &PlanBudget) -> Result<Self> {
        Self::allocate_with(count, header, budget, |buffer, count| {
            buffer
                .try_reserve_exact(count)
                .map_err(|_| LayoutError::Allocation)
        })
    }

    // Private seam for synthetic allocator surplus/failure controls. Production
    // always calls new, which allocates once from an empty Vec.
    fn allocate_with(
        count: usize,
        header: usize,
        budget: &PlanBudget,
        allocate: impl FnOnce(&mut Vec<T>, usize) -> Result<()>,
    ) -> Result<Self> {
        ready(budget)?;
        let base = budget
            .reserve_bytes(storage_bytes::<T>(header, count)?)
            .map_err(|_| LayoutError::Resource)?;
        let mut staged = Self {
            buffer: Vec::new(),
            base: Some(base),
            surplus: None,
        };
        allocate(&mut staged.buffer, count)?;
        let excess = staged
            .buffer
            .capacity()
            .checked_sub(count)
            .and_then(|capacity| capacity.checked_mul(size_of::<T>()))
            .ok_or(LayoutError::StorageOverflow)?;
        if excess != 0 {
            staged.surplus = Some(
                budget
                    .reserve_bytes(excess)
                    .map_err(|_| LayoutError::Resource)?,
            );
        }
        // No fill before reconciliation. Failure discards allocation before
        // releasing leases. Allocator-rounded transient peak is not prepaid.
        ready(budget)?;
        Ok(staged)
    }

    fn push(&mut self, value: T) -> Result<()> {
        if self.buffer.len() == self.buffer.capacity() {
            return Err(LayoutError::InvalidLayout);
        }
        self.buffer.push(value);
        Ok(())
    }

    fn seal(mut self) -> Result<(Vec<T>, RetainedStorage, Option<RetainedStorage>)> {
        let base = self
            .base
            .take()
            .ok_or(LayoutError::InvalidLayout)?
            .retain()
            .map_err(|_| LayoutError::Resource)?;
        let surplus = self
            .surplus
            .take()
            .map(PlanReservation::retain)
            .transpose()
            .map_err(|_| LayoutError::Resource)?;
        Ok((std::mem::take(&mut self.buffer), base, surplus))
    }
}

impl<T> Drop for Staged<T> {
    fn drop(&mut self) {
        // Explicit on success and all errors, including surplus rejection.
        drop(std::mem::take(&mut self.buffer));
    }
}

/// Complete private grammar identity, with immutable paid-storage evidence.
/// No Clone: a second owned allocation requires fresh admission.
#[derive(Debug)]
pub struct ClosedTypeKey {
    words: Vec<u64>,
    storage: RetainedStorage,
    surplus: Option<RetainedStorage>,
}

impl ClosedTypeKey {
    pub fn decode(words: &[u64], budget: &PlanBudget) -> Result<Self> {
        scan(words, budget)?;
        // Temporary staging metadata is separate from the eventual key header.
        let _staging = budget
            .reserve_bytes(size_of::<Staged<u64>>())
            .map_err(|_| LayoutError::Resource)?;
        let mut staged = Staged::new(words.len(), size_of::<Self>(), budget)?;
        work(budget, words.len())?;
        for &word in words {
            staged.push(word)?;
        }
        ready(budget)?;
        let (words, storage, surplus) = staged.seal()?;
        Ok(Self {
            words,
            storage,
            surplus,
        })
    }

    pub fn words(&self) -> &[u64] {
        &self.words
    }

    pub fn capacity_words(&self) -> usize {
        self.words.capacity()
    }

    pub fn storage(&self) -> &RetainedStorage {
        &self.storage
    }

    pub fn surplus_storage(&self) -> Option<&RetainedStorage> {
        self.surplus.as_ref()
    }

    fn validate(&self, budget: &PlanBudget) -> Result<Scan> {
        ready(budget)?;
        for receipt in std::iter::once(&self.storage).chain(self.surplus.as_ref()) {
            if !receipt.authority().same_authority(budget.authority()) {
                return Err(LayoutError::WrongAuthority);
            }
        }
        if self.words.len() < 3 {
            return Err(LayoutError::InvalidEncoding);
        }
        let base = storage_bytes::<u64>(size_of::<Self>(), self.words.len())?;
        let actual = storage_bytes::<u64>(size_of::<Self>(), self.words.capacity())?;
        let excess = actual.checked_sub(base).ok_or(LayoutError::InvalidLayout)?;
        if self.storage.bytes() != base
            || match &self.surplus {
                Some(receipt) => excess == 0 || receipt.bytes() != excess,
                None => excess != 0,
            }
            || self
                .storage
                .bytes()
                .checked_add(self.surplus.as_ref().map_or(0, RetainedStorage::bytes))
                != Some(actual)
        {
            return Err(LayoutError::InvalidLayout);
        }
        scan(self.words(), budget)
    }
}

#[derive(Clone, Copy)]
struct Node {
    tag: u64,
    payload: usize,
    length: usize,
}

fn node(words: &[u64], at: usize) -> Result<Node> {
    let tag = *words.get(at).ok_or(LayoutError::InvalidEncoding)?;
    let length_at = at.checked_add(1).ok_or(LayoutError::InvalidEncoding)?;
    let length = usize::try_from(*words.get(length_at).ok_or(LayoutError::InvalidEncoding)?)
        .map_err(|_| LayoutError::InvalidEncoding)?;
    let payload = at.checked_add(2).ok_or(LayoutError::InvalidEncoding)?;
    // Unary grammar: every nested node ends at the exact end of the key.
    if payload.checked_add(length) != Some(words.len()) {
        return Err(LayoutError::InvalidEncoding);
    }
    Ok(Node {
        tag,
        payload,
        length,
    })
}

#[derive(Clone, Copy)]
struct Scan {
    arrays: usize,
}

fn scan(words: &[u64], budget: &PlanBudget) -> Result<Scan> {
    work(budget, words.len())?;
    if words.first() != Some(&GRAMMAR_VERSION) {
        return Err(LayoutError::UnsupportedGrammarVersion);
    }
    let mut at = 1;
    let mut depth = 1_usize;
    let mut arrays = 0_usize;
    let mut prefix = true;
    loop {
        work(budget, 1)?;
        budget
            .check_depth(depth)
            .map_err(|_| LayoutError::Resource)?;
        let n = node(words, at)?;
        let next = match n.tag {
            0 if n.length == 1 => {
                Scalar::decode(words[n.payload])?;
                return Ok(Scan { arrays });
            }
            1 if n.length == 0 => return Ok(Scan { arrays }),
            2..=5 if n.length >= 2 => {
                prefix = false;
                n.payload
            }
            6 if n.length >= 3 => {
                if prefix {
                    work(budget, 1)?;
                    arrays = arrays.checked_add(1).ok_or(LayoutError::StorageOverflow)?;
                }
                n.payload
                    .checked_add(1)
                    .ok_or(LayoutError::InvalidEncoding)?
            }
            _ => return Err(LayoutError::InvalidEncoding),
        };
        if next <= at || next >= words.len() {
            return Err(LayoutError::InvalidEncoding);
        }
        at = next;
        depth = depth.checked_add(1).ok_or(LayoutError::StorageOverflow)?;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrayElement {
    /// Child node offset in the parent key. Its exact identity is the grammar
    /// version plus the remaining suffix, not this offset as a global ID.
    pub encoding_start: usize,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Computed {
    root: Shape,
    element: Option<ArrayElement>,
}

/// Immutable standalone result. Not a registered graph fact or source proof.
#[derive(Debug)]
pub struct ObjectLayout {
    key: ClosedTypeKey,
    target: TargetContract,
    computed: Computed,
    storage: RetainedStorage,
}

impl ObjectLayout {
    /// Consumes a paid private key. Pointer traversal stops before its pointee.
    pub fn calculate(
        key: ClosedTypeKey,
        target: TargetContract,
        budget: &PlanBudget,
    ) -> Result<Self> {
        ready(budget)?;
        target.validate()?;
        let scanned = key.validate(budget)?;
        // The moved key is already paid. Admit only additional owned fields
        // and padding, not a second ledger charge for the same key bytes.
        let additional_bytes = size_of::<Self>()
            .checked_sub(size_of::<ClosedTypeKey>())
            .ok_or(LayoutError::InvalidLayout)?;
        let paid = budget
            .reserve_bytes(additional_bytes)
            .map_err(|_| LayoutError::Resource)?;
        let computed = compute(&key, scanned, budget)?;
        ready(budget)?;
        Ok(Self {
            key,
            target,
            computed,
            storage: paid.retain().map_err(|_| LayoutError::Resource)?,
        })
    }

    pub fn shape(&self) -> Shape {
        self.computed.root
    }

    pub fn array_element(&self) -> Option<ArrayElement> {
        self.computed.element
    }

    pub fn key(&self) -> &ClosedTypeKey {
        &self.key
    }

    pub fn target(&self) -> TargetContract {
        self.target
    }

    pub fn storage(&self) -> &RetainedStorage {
        &self.storage
    }

    /// Accounting authority and exact structural identity are separate checks.
    /// Neither this check nor the private grammar grants source admission.
    pub fn validate_for(
        &self,
        expected_words: &[u64],
        expected_target: TargetContract,
        budget: &PlanBudget,
    ) -> Result<()> {
        ready(budget)?;
        expected_target.validate()?;
        if !self.storage.authority().same_authority(budget.authority())
            || !self
                .key
                .storage
                .authority()
                .same_authority(budget.authority())
        {
            return Err(LayoutError::WrongAuthority);
        }
        if self.target != expected_target {
            return Err(LayoutError::WrongTarget);
        }
        work(budget, expected_words.len())?;
        if self.key.words() != expected_words {
            return Err(LayoutError::WrongType);
        }
        let additional_bytes = size_of::<Self>()
            .checked_sub(size_of::<ClosedTypeKey>())
            .ok_or(LayoutError::InvalidLayout)?;
        if self.storage.bytes() != additional_bytes {
            return Err(LayoutError::InvalidLayout);
        }
        let scanned = self.key.validate(budget)?;
        let _scratch = budget
            .reserve_bytes(size_of::<Computed>())
            .map_err(|_| LayoutError::Resource)?;
        if compute(&self.key, scanned, budget)? != self.computed {
            return Err(LayoutError::InvalidLayout);
        }
        ready(budget)
    }
}

#[derive(Clone, Copy)]
struct ArrayFrame {
    length: u64,
    child_start: usize,
}

fn compute(key: &ClosedTypeKey, scanned: Scan, budget: &PlanBudget) -> Result<Computed> {
    let mut arrays = Staged::new(scanned.arrays, size_of::<Staged<ArrayFrame>>(), budget)?;
    let mut at = 1;
    let mut root = loop {
        work(budget, 1)?;
        let n = node(key.words(), at)?;
        match n.tag {
            0 => break Scalar::decode(key.words[n.payload])?.shape(),
            1 => {
                break Shape {
                    size: 0,
                    alignment: 1,
                    stride: 0,
                    representation: Representation::Unit,
                    validity: Validity::Unit,
                };
            }
            2..=5 => {
                break Shape {
                    size: 8,
                    alignment: 8,
                    stride: 8,
                    representation: Representation::ThinPointer(match n.tag {
                        2 => PointerKind::RawConst,
                        3 => PointerKind::RawMut,
                        4 => PointerKind::SharedReference,
                        _ => PointerKind::MutableReference,
                    }),
                    validity: if n.tag <= 3 {
                        Validity::RawPointer
                    } else {
                        Validity::Reference
                    },
                };
            }
            6 if arrays.buffer.len() < scanned.arrays => {
                work(budget, 1)?;
                let child_start = n
                    .payload
                    .checked_add(1)
                    .ok_or(LayoutError::InvalidEncoding)?;
                arrays.push(ArrayFrame {
                    length: key.words[n.payload],
                    child_start,
                })?;
                at = child_start;
            }
            _ => return Err(LayoutError::InvalidEncoding),
        }
    };
    let mut element = None;
    if arrays.buffer.len() != scanned.arrays {
        return Err(LayoutError::InvalidLayout);
    }
    for frame in arrays.buffer.iter().rev() {
        work(budget, 1)?;
        element = Some(ArrayElement {
            encoding_start: frame.child_start,
            shape: root,
        });
        let size = root
            .size
            .checked_mul(frame.length)
            .ok_or(LayoutError::SizeOverflow)?;
        if size > i64::MAX as u64 {
            return Err(LayoutError::ObjectTooLarge);
        }
        root = Shape {
            size,
            alignment: root.alignment,
            stride: size,
            representation: Representation::Array {
                length: frame.length,
            },
            validity: Validity::Elements,
        };
    }
    Ok(Computed { root, element })
}

#[cfg(test)]
mod forged_tests {
    use super::*;

    #[test]
    fn zero_prefix_and_zst_staging_charge_wrapper_once_without_heap_bytes() {
        let budget = PlanBudget::default();
        let header = size_of::<Staged<ArrayFrame>>();
        let empty = Staged::<ArrayFrame>::new(0, header, &budget).unwrap();
        assert_eq!(empty.buffer.capacity(), 0);
        assert!(empty.surplus.is_none());
        assert_eq!(budget.usage_snapshot().bytes, header);
        drop(empty);
        assert_eq!(budget.usage_snapshot().bytes, 0);
        let header = size_of::<Staged<()>>();
        let mut zst = Staged::<()>::new(17, header, &budget).unwrap();
        assert!(zst.surplus.is_none());
        for _ in 0..17 {
            zst.push(()).unwrap();
        }
        assert_eq!(budget.usage_snapshot().bytes, header);
        drop(zst);
        assert_eq!(budget.usage_snapshot().bytes, 0);
    }

    #[test]
    fn storage_arithmetic_and_allocator_errors_are_not_faked_budget_exhaustion() {
        assert_eq!(
            storage_bytes::<u64>(0, usize::MAX),
            Err(LayoutError::StorageOverflow)
        );
        assert_eq!(
            storage_bytes::<u8>(usize::MAX, 1),
            Err(LayoutError::StorageOverflow)
        );
        assert_eq!(storage_bytes::<()>(7, usize::MAX), Ok(7));
        let budget = PlanBudget::default();
        assert_eq!(
            Staged::<u64>::new(usize::MAX, 0, &budget).unwrap_err(),
            LayoutError::StorageOverflow
        );
        assert_eq!(budget.usage_snapshot().bytes, 0);
        assert_eq!(
            Staged::<u8>::allocate_with(1, 0, &budget, |_, _| Err(LayoutError::Allocation))
                .unwrap_err(),
            LayoutError::Allocation
        );
        assert_eq!(budget.usage_snapshot().bytes, 0);
        assert!(budget.first_resource_failure().is_none());
        let empty = Staged::<u8>::new(0, 0, &budget).unwrap();
        drop(empty);
    }

    #[test]
    fn surplus_is_admitted_before_fill_and_discarded_on_rejection() {
        let budget = PlanBudget::default();
        let header = size_of::<Staged<u64>>();
        let mut staged = Staged::<u64>::allocate_with(1, header, &budget, |buffer, n| {
            buffer
                .try_reserve_exact(n + 7)
                .map_err(|_| LayoutError::Allocation)
        })
        .unwrap();
        let actual = header + staged.buffer.capacity() * size_of::<u64>();
        assert_eq!(budget.usage_snapshot().bytes, actual);
        assert!(staged.surplus.is_some());
        assert!(staged.buffer.is_empty());
        staged.push(42).unwrap();
        drop(staged);
        assert_eq!(budget.usage_snapshot().bytes, 0);

        let budget = PlanBudget::new(super::super::ir::PlanLimits {
            bytes: header + size_of::<u64>(),
            ..super::super::ir::PlanLimits::default()
        });
        let called = std::cell::Cell::new(false);
        assert_eq!(
            Staged::<u64>::allocate_with(1, header, &budget, |buffer, n| {
                called.set(true);
                buffer
                    .try_reserve_exact(n + 7)
                    .map_err(|_| LayoutError::Allocation)
            })
            .unwrap_err(),
            LayoutError::Resource
        );
        assert!(called.get());
        assert_eq!(budget.usage_snapshot().bytes, 0);
        assert!(budget.first_resource_failure().is_some());
        assert_eq!(
            Staged::<u64>::new(0, 0, &budget).unwrap_err(),
            LayoutError::Resource
        );
        assert_eq!(
            ClosedTypeKey::decode(&[], &budget).unwrap_err(),
            LayoutError::Resource
        );
    }

    #[test]
    fn old_and_new_full_allocations_overlap_and_preflight_precedes_allocator() {
        let budget = PlanBudget::default();
        let header = size_of::<Staged<u64>>();
        let old = Staged::<u64>::new(8, header, &budget).unwrap();
        let old_bytes = budget.usage_snapshot().bytes;
        let replacement = Staged::<u64>::new(9, header, &budget).unwrap();
        let new_bytes = header + replacement.buffer.capacity() * size_of::<u64>();
        assert_eq!(budget.usage_snapshot().bytes, old_bytes + new_bytes);
        drop(replacement);
        assert_eq!(budget.usage_snapshot().bytes, old_bytes);
        let requested = header + 9 * size_of::<u64>();
        let blocker = budget
            .reserve_bytes(budget.limits().bytes - old_bytes - requested + 1)
            .unwrap();
        let before = budget.usage_snapshot().bytes;
        let called = std::cell::Cell::new(false);
        assert_eq!(
            Staged::<u64>::allocate_with(9, header, &budget, |_, _| {
                called.set(true);
                Ok(())
            })
            .unwrap_err(),
            LayoutError::Resource
        );
        assert!(!called.get());
        assert_eq!(budget.usage_snapshot().bytes, before);
        drop(blocker);
        drop(old);
        assert_eq!(budget.usage_snapshot().bytes, 0);
    }

    #[test]
    fn initialized_storage_is_destroyed_before_scratch_admission_refunds() {
        #[derive(Debug)]
        struct Probe<'a> {
            budget: &'a PlanBudget,
            paid: usize,
            destroyed: &'a std::cell::Cell<bool>,
        }
        impl Drop for Probe<'_> {
            fn drop(&mut self) {
                assert!(self.budget.usage_snapshot().bytes >= self.paid);
                self.destroyed.set(true);
            }
        }
        let budget = PlanBudget::default();
        let destroyed = std::cell::Cell::new(false);
        let mut staged =
            Staged::<Probe<'_>>::new(1, size_of::<Staged<Probe<'_>>>(), &budget).unwrap();
        let paid = budget.usage_snapshot().bytes;
        staged
            .push(Probe {
                budget: &budget,
                paid,
                destroyed: &destroyed,
            })
            .unwrap();
        assert!(budget.charge_work(usize::MAX).is_err());
        assert_eq!(ready(&budget), Err(LayoutError::Resource));
        drop(staged);
        assert!(destroyed.get());
        assert_eq!(budget.usage_snapshot().bytes, 0);
        assert!(budget.first_resource_failure().is_some());
    }

    #[test]
    fn retained_surplus_has_exact_authority_and_is_not_reusable_credit() {
        let budget = PlanBudget::default();
        let words = [1, 0, 1, Scalar::U8 as u64];
        let _metadata = budget.reserve_bytes(size_of::<Staged<u64>>()).unwrap();
        let mut staged = Staged::<u64>::allocate_with(
            words.len(),
            size_of::<ClosedTypeKey>(),
            &budget,
            |buffer, n| {
                buffer
                    .try_reserve_exact(n + 7)
                    .map_err(|_| LayoutError::Allocation)
            },
        )
        .unwrap();
        work(&budget, words.len()).unwrap();
        for word in words {
            staged.push(word).unwrap();
        }
        let (words, storage, surplus) = staged.seal().unwrap();
        let mut key = ClosedTypeKey {
            words,
            storage,
            surplus,
        };
        assert!(key.validate(&budget).is_ok());
        let paid = budget.usage_snapshot().bytes;
        let cloned = key.surplus.as_ref().unwrap().clone();
        drop(cloned);
        assert_eq!(budget.usage_snapshot().bytes, paid);
        let original = key.surplus.take().unwrap();
        assert!(matches!(
            key.validate(&budget),
            Err(LayoutError::InvalidLayout)
        ));
        let foreign = PlanBudget::default();
        key.surplus = Some(
            foreign
                .reserve_bytes(original.bytes())
                .unwrap()
                .retain()
                .unwrap(),
        );
        assert!(matches!(
            key.validate(&budget),
            Err(LayoutError::WrongAuthority)
        ));
        key.surplus = Some(
            budget
                .reserve_bytes(original.bytes() + 1)
                .unwrap()
                .retain()
                .unwrap(),
        );
        assert!(matches!(
            key.validate(&budget),
            Err(LayoutError::InvalidLayout)
        ));
        key.surplus = Some(original);
        assert!(key.validate(&budget).is_ok());
        let before = budget.usage_snapshot().bytes;
        drop(key);
        assert_eq!(budget.usage_snapshot().bytes, before);
    }

    #[test]
    fn complete_scan_counts_only_root_array_frontier() {
        let budget = PlanBudget::default();
        let words = [1, 6, 9, 3, 2, 6, 6, 4, u64::MAX, 0, 1, Scalar::U128 as u64];
        assert_eq!(scan(&words, &budget).unwrap().arrays, 1);
        let key = ClosedTypeKey::decode(&words, &budget).unwrap();
        let layout =
            ObjectLayout::calculate(key, TargetContract::LINUX_X86_64_V1, &budget).unwrap();
        assert_eq!(layout.shape().size, 24);
        let mut malformed = words;
        malformed[11] = 999;
        assert_eq!(
            ClosedTypeKey::decode(&malformed, &budget).unwrap_err(),
            LayoutError::InvalidEncoding
        );
    }

    #[test]
    fn calculate_rejects_forged_key_bounds_grammar_and_receipts_without_panicking() {
        let budget = PlanBudget::default();
        let words = [1, 0, 1, Scalar::U8 as u64];
        for (field, value, expected) in [
            (0, 2, LayoutError::InvalidEncoding),
            (1, 2, LayoutError::UnsupportedGrammarVersion),
            (2, 0, LayoutError::InvalidEncoding),
            (3, 999, LayoutError::InvalidEncoding),
        ] {
            let mut key = ClosedTypeKey::decode(&words, &budget).unwrap();
            match field {
                0 => key.words.truncate(value as usize),
                1 => key.words[0] = value,
                2 => key.words[2] = value,
                _ => key.words[3] = value,
            }
            assert_eq!(
                ObjectLayout::calculate(key, TargetContract::LINUX_X86_64_V1, &budget).unwrap_err(),
                expected
            );
        }
        let mut key = ClosedTypeKey::decode(&words, &budget).unwrap();
        key.storage = budget.reserve_bytes(1).unwrap().retain().unwrap();
        assert_eq!(
            ObjectLayout::calculate(key, TargetContract::LINUX_X86_64_V1, &budget).unwrap_err(),
            LayoutError::InvalidLayout
        );
        let pointer = [1, 2, 3, 0, 1, Scalar::U8 as u64];
        let mut key = ClosedTypeKey::decode(&pointer, &budget).unwrap();
        key.words[5] = 999;
        assert_eq!(
            ObjectLayout::calculate(key, TargetContract::LINUX_X86_64_V1, &budget).unwrap_err(),
            LayoutError::InvalidEncoding
        );
    }

    #[test]
    fn shape_child_and_receipt_tampering_are_independently_rejected() {
        let budget = PlanBudget::default();
        let words = [1, 6, 4, 3, 0, 1, Scalar::Bool as u64];
        let mut layout = ObjectLayout::calculate(
            ClosedTypeKey::decode(&words, &budget).unwrap(),
            TargetContract::LINUX_X86_64_V1,
            &budget,
        )
        .unwrap();
        let original = layout.computed;
        for field in 0..7 {
            match field {
                0 => layout.computed.root.size += 1,
                1 => layout.computed.root.alignment *= 2,
                2 => layout.computed.root.stride += 1,
                3 => layout.computed.root.validity = Validity::AllBits,
                4 => layout.computed.element.as_mut().unwrap().encoding_start += 1,
                5 => layout.computed.element.as_mut().unwrap().shape.validity = Validity::AllBits,
                _ => layout.computed.element.as_mut().unwrap().shape.size += 1,
            }
            assert_eq!(
                layout.validate_for(&words, TargetContract::LINUX_X86_64_V1, &budget),
                Err(LayoutError::InvalidLayout)
            );
            layout.computed = original;
        }
        layout.storage = budget.reserve_bytes(1).unwrap().retain().unwrap();
        assert_eq!(
            layout.validate_for(&words, TargetContract::LINUX_X86_64_V1, &budget),
            Err(LayoutError::InvalidLayout)
        );
    }

    #[test]
    fn forged_shape_and_target_are_not_authorized_by_receipts() {
        let budget = PlanBudget::default();
        let words = [1, 0, 1, Scalar::U8 as u64];
        let mut layout = ObjectLayout::calculate(
            ClosedTypeKey::decode(&words, &budget).unwrap(),
            TargetContract::LINUX_X86_64_V1,
            &budget,
        )
        .unwrap();
        layout.computed.root.size = 8;
        assert_eq!(
            layout.validate_for(&words, TargetContract::LINUX_X86_64_V1, &budget),
            Err(LayoutError::InvalidLayout)
        );
        layout.computed.root.size = 1;
        layout.target.version = 2;
        assert_eq!(
            layout.validate_for(&words, TargetContract::LINUX_X86_64_V1, &budget),
            Err(LayoutError::WrongTarget)
        );
    }
}
