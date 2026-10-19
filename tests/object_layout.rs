//! Standalone calculator controls, NOT native compiler/source/FFI acceptance.
//! The module is intentionally not added to compiler/mod.rs.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler::ir;
#[path = "../src/compiler/object_layout.rs"]
mod object_layout;

use ir::{PlanBudget, PlanLimits};
use object_layout::{
    Architecture, ClosedTypeKey, GRAMMAR_VERSION, LayoutError, ObjectLayout, OperatingSystem,
    PointerKind, Representation, Scalar, TargetContract, Validity,
};
use std::mem::{align_of, size_of};

const TARGET: TargetContract = TargetContract::LINUX_X86_64_V1;

fn scalar(value: Scalar) -> Vec<u64> {
    vec![GRAMMAR_VERSION, 0, 1, value as u64]
}

fn unit() -> Vec<u64> {
    vec![GRAMMAR_VERSION, 1, 0]
}

fn array(length: u64, element: &[u64]) -> Vec<u64> {
    let mut words = vec![GRAMMAR_VERSION, 6, element.len() as u64, length];
    words.extend_from_slice(&element[1..]);
    words
}

fn pointer(tag: u64, pointee: &[u64]) -> Vec<u64> {
    let mut words = vec![GRAMMAR_VERSION, tag, (pointee.len() - 1) as u64];
    words.extend_from_slice(&pointee[1..]);
    words
}

fn calculate(words: &[u64], budget: &PlanBudget) -> Result<ObjectLayout, LayoutError> {
    ObjectLayout::calculate(ClosedTypeKey::decode(words, budget)?, TARGET, budget)
}

fn rust_control<T>(words: &[u64]) {
    let budget = PlanBudget::default();
    let layout = calculate(words, &budget).unwrap();
    let shape = layout.shape();
    assert_eq!(shape.size, size_of::<T>() as u64, "{words:?}");
    assert_eq!(shape.alignment, align_of::<T>() as u64, "{words:?}");
    assert_eq!(shape.stride, size_of::<T>() as u64, "{words:?}");
    assert!(shape.alignment.is_power_of_two());
    assert_eq!(shape.size % shape.alignment, 0);
    assert_eq!(layout.key().words(), words);
    assert_eq!(layout.target(), TARGET);
    layout.validate_for(words, TARGET, &budget).unwrap();
}

#[test]
fn all_scalar_layouts_match_bootstrap_rust_not_private_carriers() {
    macro_rules! controls {
        ($($scalar:ident => $ty:ty),+ $(,)?) => {$(
            rust_control::<$ty>(&scalar(Scalar::$scalar));
            rust_control::<[$ty; 0]>(&array(0, &scalar(Scalar::$scalar)));
            rust_control::<[$ty; 3]>(&array(3, &scalar(Scalar::$scalar)));
            rust_control::<[[$ty; 3]; 2]>(&array(2, &array(3, &scalar(Scalar::$scalar))));
        )+};
    }
    controls! {
        I8 => i8, U8 => u8, I16 => i16, U16 => u16,
        I32 => i32, U32 => u32, I64 => i64, U64 => u64,
        Isize => isize, Usize => usize, I128 => i128, U128 => u128,
        F32 => f32, F64 => f64, Bool => bool, Char => char,
    }
}

#[test]
fn unit_empty_arrays_and_zero_sized_stride_match_rust() {
    rust_control::<()>(&unit());
    rust_control::<[(); 0]>(&array(0, &unit()));
    rust_control::<[(); 9]>(&array(9, &unit()));
    rust_control::<[[(); 9]; 7]>(&array(7, &array(9, &unit())));
    rust_control::<[[u128; 0]; 9]>(&array(9, &array(0, &scalar(Scalar::U128))));
    // Zero length preserves the element alignment, not alignment one.
    let budget = PlanBudget::default();
    let layout = calculate(&array(0, &scalar(Scalar::U128)), &budget).unwrap();
    assert_eq!(layout.shape().size, 0);
    assert_eq!(layout.shape().alignment, 16);
    assert_eq!(layout.array_element().unwrap().shape.size, 16);
    assert_eq!(layout.array_element().unwrap().encoding_start, 4);
    // A huge count of admitted ZSTs requires no multiplication overflow.
    let layout = calculate(&array(u64::MAX, &unit()), &budget).unwrap();
    assert_eq!(layout.shape().stride, 0);
}

#[test]
fn thin_pointer_kinds_and_exact_pointee_identity_are_preserved() {
    let pointee = array(3, &scalar(Scalar::U8));
    rust_control::<*const [u8; 3]>(&pointer(2, &pointee));
    rust_control::<*mut [u8; 3]>(&pointer(3, &pointee));
    rust_control::<&[u8; 3]>(&pointer(4, &pointee));
    rust_control::<&mut [u8; 3]>(&pointer(5, &pointee));
    rust_control::<[*const [u8; 3]; 5]>(&array(5, &pointer(2, &pointee)));
    for (tag, kind, validity) in [
        (2, PointerKind::RawConst, Validity::RawPointer),
        (3, PointerKind::RawMut, Validity::RawPointer),
        (4, PointerKind::SharedReference, Validity::Reference),
        (5, PointerKind::MutableReference, Validity::Reference),
    ] {
        let budget = PlanBudget::default();
        let words = pointer(tag, &pointee);
        let layout = calculate(&words, &budget).unwrap();
        assert_eq!(
            layout.shape().representation,
            Representation::ThinPointer(kind)
        );
        assert_eq!(layout.shape().validity, validity);
        assert!(layout.array_element().is_none());
        assert_eq!(
            layout.validate_for(&pointer(tag, &unit()), TARGET, &budget),
            Err(LayoutError::WrongType)
        );
        for other in 2..=5 {
            if other != tag {
                assert_eq!(
                    layout.validate_for(&pointer(other, &pointee), TARGET, &budget),
                    Err(LayoutError::WrongType)
                );
            }
        }
    }
}

#[test]
fn pointer_calculation_never_calculates_its_validated_pointee() {
    let overflowing = array(u64::MAX, &scalar(Scalar::U128));
    let budget = PlanBudget::default();
    assert_eq!(
        calculate(&overflowing, &budget).unwrap_err(),
        LayoutError::SizeOverflow
    );
    let pointer_words = pointer(2, &overflowing);
    let key = ClosedTypeKey::decode(&pointer_words, &budget).unwrap();
    let before = budget.usage_snapshot().work;
    let layout = ObjectLayout::calculate(key, TARGET, &budget).unwrap();
    let complex_work = budget.usage_snapshot().work - before;
    assert_eq!(layout.shape().size, 8);
    layout
        .validate_for(&pointer_words, TARGET, &budget)
        .unwrap();
    let key = ClosedTypeKey::decode(&pointer(2, &unit()), &budget).unwrap();
    let before = budget.usage_snapshot().work;
    ObjectLayout::calculate(key, TARGET, &budget).unwrap();
    // Admission rechecks all words and grammar nodes (three versus two here). The
    // actual pointer-layout walk is still one leaf and never visits pointee
    // layout, whose overflowing array would otherwise fail.
    assert_eq!(complex_work, budget.usage_snapshot().work - before + 5);
    // No pointee-layout demand is NOT permission to skip grammar validation.
    let mut malformed = pointer_words;
    *malformed.last_mut().unwrap() = 999;
    assert_eq!(
        ClosedTypeKey::decode(&malformed, &budget).unwrap_err(),
        LayoutError::InvalidEncoding
    );
}

#[test]
fn restricted_validity_is_not_lost_in_layout_or_arrays() {
    for (ty, validity) in [
        (Scalar::Bool, Validity::Bool),
        (Scalar::Char, Validity::UnicodeScalar),
    ] {
        let budget = PlanBudget::default();
        let words = scalar(ty);
        assert_eq!(
            calculate(&words, &budget).unwrap().shape().validity,
            validity
        );
        let layout = calculate(&array(4, &words), &budget).unwrap();
        assert_eq!(layout.shape().validity, Validity::Elements);
        assert_eq!(layout.array_element().unwrap().shape.validity, validity);
    }
}

#[test]
fn malformed_noncanonical_and_unsupported_type_keys_reject() {
    let budget = PlanBudget::default();
    for words in [
        vec![1],
        vec![1, 0],
        vec![1, 0, 0],
        vec![1, 0, 1, 999],
        vec![1, 1, 1, 0],
        vec![1, 99, 0],
        vec![1, 6, 0],
        vec![1, 2, 0],
        vec![1, 3, 1, 1],
        vec![1, 0, u64::MAX],
        vec![1, 1, 0, 1, 0],
    ] {
        assert_eq!(
            ClosedTypeKey::decode(&words, &budget).unwrap_err(),
            LayoutError::InvalidEncoding,
            "{words:?}"
        );
    }
    assert_eq!(
        ClosedTypeKey::decode(&[2, 1, 0], &budget).unwrap_err(),
        LayoutError::UnsupportedGrammarVersion
    );
    assert_eq!(
        ClosedTypeKey::decode(&vec![1; 1024], &budget).unwrap_err(),
        LayoutError::InvalidEncoding
    );
    // All proper truncations and length mutations of this nested key reject.
    let words = array(2, &pointer(3, &array(7, &scalar(Scalar::Char))));
    for cut in 1..words.len() {
        assert!(ClosedTypeKey::decode(&words[..cut], &budget).is_err());
    }
    for at in [2, 5, 7, 10] {
        let mut changed = words.clone();
        changed[at] += 1;
        assert!(
            ClosedTypeKey::decode(&changed, &budget).is_err(),
            "{changed:?}"
        );
    }
}

#[test]
fn target_contract_and_version_are_exact_and_not_host_fallbacks() {
    let budget = PlanBudget::default();
    for target in [
        TargetContract {
            architecture: Architecture::Aarch64,
            ..TARGET
        },
        TargetContract {
            operating_system: OperatingSystem::Windows,
            ..TARGET
        },
    ] {
        let key = ClosedTypeKey::decode(&unit(), &budget).unwrap();
        assert_eq!(
            ObjectLayout::calculate(key, target, &budget).unwrap_err(),
            LayoutError::UnsupportedTarget
        );
    }
    for version in [0, 2, u32::MAX] {
        let target = TargetContract { version, ..TARGET };
        let key = ClosedTypeKey::decode(&unit(), &budget).unwrap();
        assert_eq!(
            ObjectLayout::calculate(key, target, &budget).unwrap_err(),
            LayoutError::UnsupportedTargetVersion
        );
    }
    let layout = calculate(&unit(), &budget).unwrap();
    assert_eq!(
        layout.validate_for(
            &unit(),
            TargetContract {
                version: 2,
                ..TARGET
            },
            &budget
        ),
        Err(LayoutError::UnsupportedTargetVersion)
    );
}

#[test]
fn arithmetic_overflow_and_target_object_bound_are_distinct() {
    let budget = PlanBudget::default();
    assert_eq!(
        calculate(&array(u64::MAX, &scalar(Scalar::U128)), &budget).unwrap_err(),
        LayoutError::SizeOverflow
    );
    assert_eq!(
        calculate(&array(1 << 63, &scalar(Scalar::U8)), &budget).unwrap_err(),
        LayoutError::ObjectTooLarge
    );
    let within = calculate(&array(i64::MAX as u64, &scalar(Scalar::U8)), &budget).unwrap();
    assert_eq!(within.shape().size, i64::MAX as u64);
    assert_eq!(within.array_element().unwrap().shape.stride, 1);
    // Even an empty outer array must not hide an overflowing element layout.
    assert_eq!(
        calculate(&array(0, &array(u64::MAX, &scalar(Scalar::U128))), &budget).unwrap_err(),
        LayoutError::SizeOverflow
    );
}

#[test]
fn actual_owned_capacity_is_paid_once_and_receipts_never_refund() {
    let budget = PlanBudget::default();
    let words = scalar(Scalar::U8);
    let key = ClosedTypeKey::decode(&words, &budget).unwrap();
    let capacity_bytes = key.capacity_words() * size_of::<u64>();
    let base_bytes = size_of::<ClosedTypeKey>() + words.len() * size_of::<u64>();
    assert_eq!(key.storage().bytes(), base_bytes);
    let surplus = key.surplus_storage().map_or(0, |receipt| receipt.bytes());
    assert_eq!(
        base_bytes + surplus,
        size_of::<ClosedTypeKey>() + capacity_bytes
    );
    assert_eq!(
        budget.usage_snapshot().bytes,
        size_of::<ClosedTypeKey>() + capacity_bytes
    );
    let address = key.words().as_ptr();
    let layout = ObjectLayout::calculate(key, TARGET, &budget).unwrap();
    assert_eq!(
        layout.key().storage().bytes() + surplus + layout.storage().bytes(),
        size_of::<ObjectLayout>() + capacity_bytes
    );
    assert_eq!(
        layout.key().words().as_ptr(),
        address,
        "calculate moves, never copies the allocation"
    );
    assert_eq!(
        budget.usage_snapshot().bytes,
        size_of::<ObjectLayout>() + capacity_bytes
    );
    let before = budget.usage_snapshot();
    let receipt = layout.storage().clone();
    drop(receipt);
    layout.validate_for(&words, TARGET, &budget).unwrap();
    assert_eq!(budget.usage_snapshot().bytes, before.bytes);
    drop(layout);
    assert_eq!(budget.usage_snapshot().bytes, before.bytes);
}

#[test]
fn equal_limits_or_receipts_do_not_authorize_foreign_keys_or_outputs() {
    let original = PlanBudget::default();
    let foreign = PlanBudget::default();
    let words = scalar(Scalar::U8);
    let key = ClosedTypeKey::decode(&words, &original).unwrap();
    assert_eq!(
        ObjectLayout::calculate(key, TARGET, &foreign).unwrap_err(),
        LayoutError::WrongAuthority
    );
    let layout = calculate(&words, &original).unwrap();
    assert_eq!(
        layout.validate_for(&words, TARGET, &foreign),
        Err(LayoutError::WrongAuthority)
    );
    assert_eq!(
        layout.validate_for(&scalar(Scalar::I8), TARGET, &original),
        Err(LayoutError::WrongType)
    );
    let shared_authority = original.clone();
    layout
        .validate_for(&words, TARGET, &shared_authority)
        .unwrap();
}

#[test]
fn work_bytes_and_depth_exhaustion_remain_sticky() {
    for limits in [
        PlanLimits {
            work: 0,
            ..PlanLimits::default()
        },
        PlanLimits {
            bytes: size_of::<ClosedTypeKey>() - 1,
            ..PlanLimits::default()
        },
        PlanLimits {
            depth: 1,
            ..PlanLimits::default()
        },
    ] {
        let budget = PlanBudget::new(limits);
        let words = pointer(2, &scalar(Scalar::U8));
        assert_eq!(
            ClosedTypeKey::decode(&words, &budget).unwrap_err(),
            LayoutError::Resource
        );
        assert!(budget.first_resource_failure().is_some());
        assert_eq!(
            ClosedTypeKey::decode(&[], &budget).unwrap_err(),
            LayoutError::Resource
        );
        assert_eq!(
            ClosedTypeKey::decode(&unit(), &budget).unwrap_err(),
            LayoutError::Resource
        );
    }
    let budget = PlanBudget::default();
    let layout = calculate(&unit(), &budget).unwrap();
    let key = ClosedTypeKey::decode(&unit(), &budget).unwrap();
    assert!(budget.charge_work(usize::MAX).is_err());
    assert_eq!(
        ObjectLayout::calculate(key, TARGET, &budget).unwrap_err(),
        LayoutError::Resource
    );
    assert_eq!(
        layout.validate_for(&unit(), TARGET, &budget),
        Err(LayoutError::Resource)
    );
    // Key admission can fit while the calculation's scratch does not.
    let budget = PlanBudget::new(PlanLimits {
        bytes: size_of::<ObjectLayout>(),
        ..PlanLimits::default()
    });
    assert_eq!(
        calculate(&unit(), &budget).unwrap_err(),
        LayoutError::Resource
    );
    assert!(budget.first_resource_failure().is_some());
}

#[test]
fn complete_pointer_suffix_depth_uses_only_caller_admission() {
    let budget = PlanBudget::default();
    let mut words = unit();
    for _ in 1..budget.limits().depth {
        words = pointer(2, &words);
    }
    let key = ClosedTypeKey::decode(&words, &budget).unwrap();
    assert_eq!(
        ObjectLayout::calculate(key, TARGET, &budget)
            .unwrap()
            .shape()
            .size,
        8
    );
    words = pointer(2, &words);
    assert_eq!(
        ClosedTypeKey::decode(&words, &budget).unwrap_err(),
        LayoutError::Resource
    );
    assert_eq!(
        ClosedTypeKey::decode(&unit(), &budget).unwrap_err(),
        LayoutError::Resource
    );
}

#[test]
fn array_frontiers_scale_with_actual_input_under_caller_depth() {
    for depth in [1, 2, 17, PlanLimits::default().depth] {
        let budget = PlanBudget::new(PlanLimits {
            depth,
            ..PlanLimits::default()
        });
        let mut words = unit();
        for _ in 1..depth {
            words = array(0, &words);
        }
        let layout = calculate(&words, &budget).unwrap();
        assert_eq!(layout.shape().size, 0);
        layout.validate_for(&words, TARGET, &budget).unwrap();
        assert_eq!(
            budget.usage_snapshot().bytes,
            size_of::<ObjectLayout>() + layout.key().capacity_words() * size_of::<u64>()
        );
        let over = array(0, &words);
        assert_eq!(
            ClosedTypeKey::decode(&over, &budget).unwrap_err(),
            LayoutError::Resource
        );
        assert!(budget.first_resource_failure().is_some());
    }
}

#[test]
fn owned_input_is_not_a_borrowed_backreference() {
    let budget = PlanBudget::default();
    let mut input = scalar(Scalar::Char);
    let key = ClosedTypeKey::decode(&input, &budget).unwrap();
    input[3] = Scalar::U8 as u64;
    drop(input);
    let layout = ObjectLayout::calculate(key, TARGET, &budget).unwrap();
    assert_eq!(layout.shape().size, 4);
    layout
        .validate_for(&scalar(Scalar::Char), TARGET, &budget)
        .unwrap();
}

#[test]
fn c_scalar_and_array_object_layout_controls() {
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    let path = base.join(format!(
        "object-layout-c-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(path);
    let source = scratch.0.join("control.c");
    let executable = scratch.0.join("control");
    fs::write(&source, r#"
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdbool.h>
#define ROW(T) do { T a[3]; printf("%zu %zu %zu %zu %zu\n", sizeof(T), _Alignof(T), sizeof(a), _Alignof(T[3]), (size_t)((unsigned char *)&a[1] - (unsigned char *)&a[0])); } while (0)
int main(void) {
    ROW(int8_t); ROW(uint8_t); ROW(int16_t); ROW(uint16_t);
    ROW(int32_t); ROW(uint32_t); ROW(int64_t); ROW(uint64_t);
    ROW(intptr_t); ROW(uintptr_t); ROW(__int128); ROW(unsigned __int128);
    ROW(float); ROW(double); ROW(_Bool); ROW(uint32_t); ROW(void *);
    return 0;
}
"#).unwrap();
    let compile = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("C layout controls require cc");
    assert!(
        compile.status.success(),
        "C compiler failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(&executable).output().unwrap();
    assert!(run.status.success(), "C control failed: {run:?}");
    let output = String::from_utf8(run.stdout).unwrap();
    let rows: Vec<Vec<u64>> = output
        .lines()
        .map(|line| {
            line.split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect()
        })
        .collect();
    let mut types: Vec<Vec<u64>> = [
        Scalar::I8,
        Scalar::U8,
        Scalar::I16,
        Scalar::U16,
        Scalar::I32,
        Scalar::U32,
        Scalar::I64,
        Scalar::U64,
        Scalar::Isize,
        Scalar::Usize,
        Scalar::I128,
        Scalar::U128,
        Scalar::F32,
        Scalar::F64,
        Scalar::Bool,
        Scalar::Char,
    ]
    .into_iter()
    .map(scalar)
    .collect();
    types.push(pointer(2, &unit()));
    assert_eq!(rows.len(), types.len());
    // C uint32_t controls char storage only, not Rust Unicode validity. C has
    // no standard unit/ZST object, so those remain bootstrap-Rust controls.
    for (words, row) in types.iter().zip(rows) {
        assert_eq!(row.len(), 5);
        let budget = PlanBudget::default();
        let scalar = calculate(words, &budget).unwrap();
        let array = calculate(&array(3, words), &budget).unwrap();
        assert_eq!(
            [
                scalar.shape().size,
                scalar.shape().alignment,
                array.shape().size,
                array.shape().alignment,
                array.array_element().unwrap().shape.stride
            ],
            row.as_slice(),
            "{words:?}"
        );
    }
}
