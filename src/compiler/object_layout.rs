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
//! Fixed buffers avoid hidden allocation capacity or relocation peaks. Their
//! complete inline capacity, including unused words, is paid before construction.
//! Only existing PlanBudget admission and immutable RetainedStorage are used.
//! Receipts prove accounting authority, never source or semantic authority.

use super::ir::{PlanBudget, RetainedStorage};
use std::mem::size_of;

pub const GRAMMAR_VERSION: u64 = 1;
const MAX_DEPTH: usize = 128;
const KEY_WORDS: usize = 1 + 3 * MAX_DEPTH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    Resource,
    KeyCapacity,
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

/// Complete private grammar identity, with immutable paid-storage evidence.
/// No Clone: duplicating the owned inline buffer requires new admission.
#[derive(Debug)]
pub struct ClosedTypeKey {
    words: [u64; KEY_WORDS],
    len: usize,
    storage: RetainedStorage,
}

impl ClosedTypeKey {
    pub fn decode(words: &[u64], budget: &PlanBudget) -> Result<Self> {
        // Check sticky failure even for malformed, empty or zero-work requests.
        work(budget, words.len())?;
        if words.first() != Some(&GRAMMAR_VERSION) {
            return Err(LayoutError::UnsupportedGrammarVersion);
        }
        if words.len() > KEY_WORDS {
            return Err(LayoutError::KeyCapacity);
        }
        validate_grammar(words, budget)?;
        let paid = budget
            .reserve_bytes(size_of::<Self>())
            .map_err(|_| LayoutError::Resource)?;
        // Zeroing unused fixed capacity and copying the used words are work.
        work(budget, KEY_WORDS + words.len())?;
        let mut owned = [0; KEY_WORDS];
        owned[..words.len()].copy_from_slice(words);
        ready(budget)?;
        Ok(Self {
            words: owned,
            len: words.len(),
            storage: paid.retain().map_err(|_| LayoutError::Resource)?,
        })
    }

    pub fn words(&self) -> &[u64] {
        &self.words[..self.len]
    }

    pub fn storage(&self) -> &RetainedStorage {
        &self.storage
    }

    fn validate(&self, budget: &PlanBudget) -> Result<()> {
        ready(budget)?;
        if !self.storage.authority().same_authority(budget.authority()) {
            return Err(LayoutError::WrongAuthority);
        }
        if self.storage.bytes() != size_of::<Self>() {
            return Err(LayoutError::InvalidLayout);
        }
        if !(3..=KEY_WORDS).contains(&self.len) {
            return Err(LayoutError::InvalidEncoding);
        }
        if self.words[0] != GRAMMAR_VERSION {
            return Err(LayoutError::UnsupportedGrammarVersion);
        }
        validate_grammar(self.words(), budget)
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
    let length = usize::try_from(*words.get(at + 1).ok_or(LayoutError::InvalidEncoding)?)
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

fn validate_grammar(words: &[u64], budget: &PlanBudget) -> Result<()> {
    let mut at = 1;
    for depth in 1..=MAX_DEPTH {
        work(budget, 1)?;
        budget
            .check_depth(depth)
            .map_err(|_| LayoutError::Resource)?;
        let n = node(words, at)?;
        match n.tag {
            0 if n.length == 1 => {
                Scalar::decode(words[n.payload])?;
                return Ok(());
            }
            1 if n.length == 0 => return Ok(()),
            2..=5 if n.length >= 2 => at = n.payload,
            6 if n.length >= 3 => at = n.payload + 1,
            _ => return Err(LayoutError::InvalidEncoding),
        }
    }
    // The same existing task budget owns depth rejection and sticky failure.
    budget
        .check_depth(MAX_DEPTH + 1)
        .map_err(|_| LayoutError::Resource)?;
    Err(LayoutError::KeyCapacity)
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
        key.validate(budget)?;
        // The moved key is already paid. Admit only additional owned fields
        // and padding, not a second ledger charge for the same key bytes.
        let additional_bytes = size_of::<Self>()
            .checked_sub(size_of::<ClosedTypeKey>())
            .ok_or(LayoutError::InvalidLayout)?;
        let paid = budget
            .reserve_bytes(additional_bytes)
            .map_err(|_| LayoutError::Resource)?;
        let computed = compute(&key, budget)?;
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
        if self.key.len > KEY_WORDS || self.key.words[0] != GRAMMAR_VERSION {
            return Err(LayoutError::InvalidEncoding);
        }
        work(budget, expected_words.len())?;
        if self.key.words() != expected_words {
            return Err(LayoutError::WrongType);
        }
        let additional_bytes = size_of::<Self>()
            .checked_sub(size_of::<ClosedTypeKey>())
            .ok_or(LayoutError::InvalidLayout)?;
        if self.key.storage.bytes() != size_of::<ClosedTypeKey>()
            || self.storage.bytes() != additional_bytes
        {
            return Err(LayoutError::InvalidLayout);
        }
        self.key.validate(budget)?;
        let _scratch = budget
            .reserve_bytes(size_of::<Computed>())
            .map_err(|_| LayoutError::Resource)?;
        if compute(&self.key, budget)? != self.computed {
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

fn compute(key: &ClosedTypeKey, budget: &PlanBudget) -> Result<Computed> {
    let _scratch = budget
        .reserve_bytes(size_of::<[ArrayFrame; MAX_DEPTH]>())
        .map_err(|_| LayoutError::Resource)?;
    work(budget, MAX_DEPTH)?;
    let mut arrays = [ArrayFrame {
        length: 0,
        child_start: 0,
    }; MAX_DEPTH];
    let mut count = 0;
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
            6 if count < MAX_DEPTH => {
                arrays[count] = ArrayFrame {
                    length: key.words[n.payload],
                    child_start: n.payload + 1,
                };
                count += 1;
                at = n.payload + 1;
            }
            _ => return Err(LayoutError::InvalidEncoding),
        }
    };
    let mut element = None;
    for frame in arrays[..count].iter().rev() {
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
    fn calculate_rejects_forged_key_bounds_grammar_and_receipts_without_panicking() {
        let budget = PlanBudget::default();
        let words = [1, 0, 1, Scalar::U8 as u64];
        for (field, value, expected) in [
            (0, (KEY_WORDS + 1) as u64, LayoutError::InvalidEncoding),
            (0, 2, LayoutError::InvalidEncoding),
            (1, 2, LayoutError::UnsupportedGrammarVersion),
            (2, 0, LayoutError::InvalidEncoding),
            (3, 999, LayoutError::InvalidEncoding),
        ] {
            let mut key = ClosedTypeKey::decode(&words, &budget).unwrap();
            match field {
                0 => key.len = value as usize,
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
