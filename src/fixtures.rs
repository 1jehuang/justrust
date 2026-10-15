//! Deterministic engine fixtures, not a Rust frontend or compiler benchmark.
use crate::{Context, Error, Key, Provider, TaskFuture};

const WORK: u32 = 1;
const WIDE: u32 = 2;
const CHAIN: u32 = 3;
const DIAMOND: u32 = 4;
const ARM: u32 = 5;
const DYNAMIC: u32 = 6;
const FIXED_POINT: u32 = 7;
const SOURCE: u32 = 10;
const SIGNATURE: u32 = 11;
const BODY: u32 = 12;
const OBJECT: u32 = 13;
const BUILD: u32 = 14;
const CHECK: u32 = 15;
const TEST: u32 = 16;
const HARNESS: u32 = 17;
const FRONTIER: u32 = 20;
const EARLY_PRODUCER: u32 = 21;
const RELEASE: u32 = 22;

#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Wide,
    Chain,
    Diamond,
    Dynamic,
    FixedPoint,
    BuildCheckTest,
    MultiOutput,
}

impl Shape {
    pub fn parse(name: &str) -> Result<Self, Error> {
        match name {
            "wide" => Ok(Self::Wide),
            "chain" => Ok(Self::Chain),
            "diamond" => Ok(Self::Diamond),
            "dynamic" => Ok(Self::Dynamic),
            "fixed-point" => Ok(Self::FixedPoint),
            "multi-output" => Ok(Self::MultiOutput),
            "build-check-test" => Ok(Self::BuildCheckTest),
            _ => Err(Error::new(format!("unknown fixture {name}"))),
        }
    }

    pub fn roots(self, size: u64, work: u64, seed: u64) -> Vec<Key> {
        let p = vec![size, work, seed];
        match self {
            Self::Wide => vec![Key::new(WIDE, p)],
            Self::Chain => vec![Key::new(CHAIN, p)],
            Self::Diamond => vec![Key::new(DIAMOND, p)],
            Self::Dynamic => vec![Key::new(DYNAMIC, p)],
            Self::FixedPoint => vec![Key::new(FIXED_POINT, p)],
            Self::MultiOutput => vec![Key::new(FRONTIER, p)],
            Self::BuildCheckTest => [BUILD, CHECK, TEST]
                .into_iter()
                .map(|kind| Key::new(kind, p.clone()))
                .collect(),
        }
    }

    /// Intentionally sequential, without engine interning or scheduling.
    pub fn reference(self, size: u64, work: u64, seed: u64) -> Vec<u64> {
        match self {
            Self::Wide => vec![(0..size).fold(0u64, |a, i| a.wrapping_add(burn(i, work, seed)))],
            Self::Chain => vec![(0..=size).fold(0u64, |a, i| a.wrapping_add(burn(i, work, seed)))],
            Self::Diamond => {
                let mut value = burn(0, work, seed);
                for depth in 1..=size {
                    value = value
                        .wrapping_mul(2)
                        .wrapping_add(burn(depth.wrapping_mul(2), work, seed))
                        .wrapping_add(burn(depth.wrapping_mul(2).wrapping_add(1), work, seed));
                }
                vec![value]
            }
            Self::Dynamic => {
                let mut state = seed;
                let mut sum = 0u64;
                for depth in (1..=size).rev() {
                    let value = burn(depth, work, state);
                    sum = sum.wrapping_add(value);
                    state = value;
                }
                vec![sum.wrapping_add(burn(0, work, state))]
            }
            Self::FixedPoint => vec![if size >= 64 {
                u64::MAX
            } else {
                (1u64 << size) - 1
            }],
            Self::MultiOutput => {
                vec![burn(size, work, seed).wrapping_add(burn(size + 1, work, seed))]
            }
            Self::BuildCheckTest => {
                let source = burn(size, work, seed);
                let signature = source.rotate_left(7);
                let body = signature.wrapping_add(burn(size + 1, work, seed));
                let object = body.rotate_left(13);
                let build = object ^ 0x0042_5549_4c44;
                let harness = signature.wrapping_add(burn(size + 2, work, seed));
                vec![build, body, build.wrapping_add(harness)]
            }
        }
    }
}

/// Deliberately CPU-bound, deterministic and identical in reference and engine.
/// Useful work is controlled by `rounds`, not by sleeps or wall-clock timing.
pub fn burn(index: u64, rounds: u64, seed: u64) -> u64 {
    let mut x = std::hint::black_box(seed ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    for i in 0..rounds {
        x = x.wrapping_add(i ^ 0xd1b5_4a32_d192_ed03);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d0_49bb_1331_11eb).rotate_left(17);
    }
    std::hint::black_box(x)
}

/// Monotone finite-lattice example. This is explicit iteration inside one
/// operation, not a claim that arbitrary recursive inference is implemented.
fn fixed_point(size: u64) -> u64 {
    let bits = size.min(64);
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let mut state = 0;
    loop {
        let next = ((state << 1) | 1) & mask;
        if next == state {
            return state;
        }
        state = next;
    }
}

pub struct FixtureProvider;

impl Provider for FixtureProvider {
    fn outputs(&self, key: &Key) -> usize {
        if key.kind == EARLY_PRODUCER { 2 } else { 1 }
    }

    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        Box::pin(async move {
            if key.kind == ARM {
                let [depth, work, seed, side] = key.params.as_slice() else {
                    return Err(Error::new("arm expects depth, work, seed, side"));
                };
                let shared = cx.request(Key::new(DIAMOND, vec![depth - 1, *work, *seed]));
                let value = cx.read(shared.get(0)).await?.wrapping_add(burn(
                    depth.wrapping_mul(2).wrapping_add(*side),
                    *work,
                    *seed,
                ));
                return cx.publish(0, value);
            }
            let [size, work, seed] = key.params.as_slice() else {
                return Err(Error::new("fixture expects size, work, seed"));
            };
            let (size, work, seed) = (*size, *work, *seed);
            let sub = |kind, n| Key::new(kind, vec![n, work, seed]);
            let value = match key.kind {
                EARLY_PRODUCER => {
                    cx.publish(0, burn(size, work, seed))?;
                    let branch = cx.clone();
                    cx.fork(Box::pin(async move {
                        let release = branch.request(Key::new(RELEASE, vec![size, work, seed]));
                        branch.read(release.get(0)).await?;
                        branch.publish(1, burn(size + 1, work, seed))
                    }))?;
                    return Ok(());
                }
                RELEASE => {
                    cx.read(cx.request(sub(EARLY_PRODUCER, size)).get(0))
                        .await?
                }
                FRONTIER => {
                    let producer = cx.request(sub(EARLY_PRODUCER, size));
                    let consumer = cx.request(sub(RELEASE, size));
                    cx.read(producer.get(1))
                        .await?
                        .wrapping_add(cx.read(consumer.get(0)).await?)
                }
                WORK | SOURCE => burn(size, work, seed),
                WIDE => {
                    // Demand the full independent frontier before waiting.
                    let inputs: Vec<_> =
                        (0..size).map(|i| cx.request(sub(WORK, i)).get(0)).collect();
                    let mut sum = 0u64;
                    for handle in inputs {
                        sum = sum.wrapping_add(cx.read(handle).await?);
                    }
                    sum
                }
                CHAIN => {
                    let own = burn(size, work, seed);
                    if size == 0 {
                        own
                    } else {
                        own.wrapping_add(cx.read(cx.request(sub(CHAIN, size - 1)).get(0)).await?)
                    }
                }
                DIAMOND => {
                    if size == 0 {
                        burn(0, work, seed)
                    } else {
                        let a = cx.request(Key::new(ARM, vec![size, work, seed, 0]));
                        let b = cx.request(Key::new(ARM, vec![size, work, seed, 1]));
                        cx.read(a.get(0))
                            .await?
                            .wrapping_add(cx.read(b.get(0)).await?)
                    }
                }
                DYNAMIC => {
                    let own = burn(size, work, seed);
                    if size == 0 {
                        own
                    } else {
                        let next = cx.request(Key::new(DYNAMIC, vec![size - 1, work, own]));
                        own.wrapping_add(cx.read(next.get(0)).await?)
                    }
                }
                FIXED_POINT => fixed_point(size),
                SIGNATURE => cx
                    .read(cx.request(sub(SOURCE, size)).get(0))
                    .await?
                    .rotate_left(7),
                BODY => cx
                    .read(cx.request(sub(SIGNATURE, size)).get(0))
                    .await?
                    .wrapping_add(burn(size + 1, work, seed)),
                OBJECT => cx
                    .read(cx.request(sub(BODY, size)).get(0))
                    .await?
                    .rotate_left(13),
                BUILD => cx.read(cx.request(sub(OBJECT, size)).get(0)).await? ^ 0x0042_5549_4c44,
                CHECK => cx.read(cx.request(sub(BODY, size)).get(0)).await?,
                HARNESS => cx
                    .read(cx.request(sub(SIGNATURE, size)).get(0))
                    .await?
                    .wrapping_add(burn(size + 2, work, seed)),
                TEST => {
                    let build = cx.request(sub(BUILD, size));
                    let harness = cx.request(sub(HARNESS, size));
                    cx.read(build.get(0))
                        .await?
                        .wrapping_add(cx.read(harness.get(0)).await?)
                }
                _ => return Err(Error::new("unknown fixture operation")),
            };
            cx.publish(0, value)
        })
    }
}
