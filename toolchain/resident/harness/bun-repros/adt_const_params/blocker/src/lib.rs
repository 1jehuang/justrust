#![feature(adt_const_params)]

use std::marker::ConstParamTy;

#[derive(PartialEq, Eq, ConstParamTy)]
enum Requirement {
    Connected,
    NotSubscriber,
    Within { low: u16, high: u16 },
    Kind(u8),
}

#[derive(PartialEq, Eq, ConstParamTy)]
struct Filter {
    mask: u32,
    strict: bool,
}

fn check<const REQUIREMENT: Requirement>(subscribed: bool, value: u16) -> bool {
    match REQUIREMENT {
        Requirement::Connected => true,
        Requirement::NotSubscriber => !subscribed,
        Requirement::Within { low, high } => low <= value && value <= high,
        Requirement::Kind(kind) => value == kind as u16,
    }
}

fn filtered<const FILTER: Filter>(bits: u32) -> u32 {
    if FILTER.strict { bits & FILTER.mask } else { bits | FILTER.mask }
}

fn weigh<const WEIGHTS: (u8, [u16; 3])>() -> u32 {
    WEIGHTS.0 as u32 * (WEIGHTS.1[0] as u32 + WEIGHTS.1[1] as u32 + WEIGHTS.1[2] as u32)
}

pub fn run() -> (bool, bool, bool, bool, u32, u32, u32) {
    (
        check::<{ Requirement::Connected }>(false, 0),
        check::<{ Requirement::NotSubscriber }>(true, 0),
        check::<{ Requirement::Within { low: 3, high: 9 } }>(false, 7),
        check::<{ Requirement::Kind(4) }>(false, 5),
        filtered::<{ Filter { mask: 0xf0, strict: true } }>(0x3c),
        filtered::<{ Filter { mask: 0x01, strict: false } }>(0x3c),
        weigh::<{ (3, [1, 20, 300]) }>(),
    )
}
