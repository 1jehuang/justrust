//! CPU-bound programs for comparing the code of one compiler with another's.
//!
//! `kernels NAME...` runs each kernel named (all of them without arguments), prints its name
//! and a checksum on stdout, which has to be the same whoever compiled it, and the
//! milliseconds it took on stderr.

use std::collections::HashMap;
use std::time::Instant;

const KERNELS: &[(&str, fn() -> u64)] = &[
    ("sieve", sieve),
    ("collatz", collatz),
    ("fnv", fnv),
    ("qsort", qsort),
    ("matmul", matmul),
    ("nbody", nbody),
    ("mandelbrot", mandelbrot),
    ("words", words),
];

fn main() {
    let asked: Vec<String> = std::env::args().skip(1).collect();
    for (name, kernel) in KERNELS {
        if !asked.is_empty() && !asked.iter().any(|it| it == name) {
            continue;
        }
        let start = Instant::now();
        let checksum = kernel();
        eprintln!("{name} {}", start.elapsed().as_millis());
        println!("{name} {checksum}");
    }
}

/// A linear congruential generator, so that every build sees the same input.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

/// How many primes there are below sixteen million: indexing and stepping through a vector.
fn sieve() -> u64 {
    let limit = 16_000_000;
    let mut composite = vec![false; limit];
    let mut count = 0;
    for n in 2..limit {
        if composite[n] {
            continue;
        }
        count += 1;
        let mut multiple = n * n;
        while multiple < limit {
            composite[multiple] = true;
            multiple += n;
        }
    }
    count
}

/// The longest Collatz chain that starts below a million: integer arithmetic
/// and branches, with no memory.
fn collatz() -> u64 {
    let mut best = (0, 0);
    for start in 1..1_000_000u64 {
        let mut n = start;
        let mut steps = 0u64;
        while n != 1 {
            n = if n % 2 == 0 { n / 2 } else { 3 * n + 1 };
            steps += 1;
        }
        if steps > best.0 {
            best = (steps, start);
        }
    }
    best.0 * 1_000_000 + best.1
}

/// FNV-1a over a buffer, many times: a tight loop over a slice iterator.
fn fnv() -> u64 {
    let mut random = Lcg(7);
    let bytes: Vec<u8> = (0..1 << 16).map(|_| random.next() as u8).collect();
    let mut hash = 0xcbf29ce484222325u64;
    for _ in 0..1000 {
        for &byte in &bytes {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    hash
}

/// Quicksort written out: recursion, swaps and comparisons on a slice.
fn qsort() -> u64 {
    fn sort(items: &mut [u32]) {
        if items.len() < 2 {
            return;
        }
        let pivot = items[items.len() / 2];
        let (mut low, mut high) = (0, items.len() - 1);
        loop {
            while items[low] < pivot {
                low += 1;
            }
            while items[high] > pivot {
                high -= 1;
            }
            if low >= high {
                break;
            }
            items.swap(low, high);
            low += 1;
            high -= 1;
        }
        let (left, right) = items.split_at_mut(low);
        sort(left);
        sort(right);
    }
    let mut random = Lcg(11);
    let mut items: Vec<u32> = (0..3_000_000).map(|_| random.next()).collect();
    sort(&mut items);
    assert!(items.windows(2).all(|pair| pair[0] <= pair[1]));
    items.iter().step_by(1000).fold(0u64, |sum, &item| sum.wrapping_mul(31).wrapping_add(item.into()))
}

/// The product of two matrices of floats kept in flat vectors.
fn matmul() -> u64 {
    let n = 320;
    let a: Vec<f64> = (0..n * n).map(|i| (i % 17) as f64 * 0.5).collect();
    let b: Vec<f64> = (0..n * n).map(|i| (i % 13) as f64 - 6.0).collect();
    let mut c = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            let mut sum = 0.0;
            for k in 0..n {
                sum += a[i * n + k] * b[k * n + j];
            }
            c[i * n + j] = sum;
        }
    }
    c.iter().sum::<f64>().to_bits()
}

#[derive(Clone, Copy)]
struct Body {
    position: [f64; 3],
    velocity: [f64; 3],
    mass: f64,
}

/// Bodies that pull on each other: floats in an array of structs.
fn nbody() -> u64 {
    let mut bodies = [
        Body { position: [0.0, 0.0, 0.0], velocity: [0.0, 0.0, 0.0], mass: 39.47 },
        Body { position: [4.84, -1.16, -0.10], velocity: [0.606, 2.81, -0.02], mass: 0.037 },
        Body { position: [8.34, 4.12, -0.4], velocity: [-1.01, 1.82, 0.008], mass: 0.011 },
        Body { position: [12.89, -15.11, -0.22], velocity: [1.08, 0.868, -0.01], mass: 0.0017 },
        Body { position: [15.37, -25.9, 0.179], velocity: [0.979, 0.594, -0.034], mass: 0.002 },
    ];
    let dt = 0.01;
    for _ in 0..600_000 {
        for i in 0..bodies.len() {
            for j in i + 1..bodies.len() {
                let mut delta = [0.0; 3];
                for axis in 0..3 {
                    delta[axis] = bodies[i].position[axis] - bodies[j].position[axis];
                }
                let distance2: f64 = delta.iter().map(|d| d * d).sum();
                let magnitude = dt / (distance2 * distance2.sqrt());
                for axis in 0..3 {
                    bodies[i].velocity[axis] -= delta[axis] * bodies[j].mass * magnitude;
                    bodies[j].velocity[axis] += delta[axis] * bodies[i].mass * magnitude;
                }
            }
        }
        for body in &mut bodies {
            for axis in 0..3 {
                body.position[axis] += dt * body.velocity[axis];
            }
        }
    }
    bodies.iter().map(|body| body.position[0].to_bits()).fold(0, |sum, bits| sum ^ bits)
}

/// How long points of a grid take to leave the Mandelbrot set.
fn mandelbrot() -> u64 {
    let (width, height, limit) = (1600, 1200, 200);
    let mut total = 0u64;
    for row in 0..height {
        for column in 0..width {
            let (cr, ci) = (column as f64 * 3.5 / width as f64 - 2.5, row as f64 * 2.0 / height as f64 - 1.0);
            let (mut zr, mut zi, mut steps) = (0.0f64, 0.0f64, 0);
            while steps < limit && zr * zr + zi * zi <= 4.0 {
                (zr, zi) = (zr * zr - zi * zi + cr, 2.0 * zr * zi + ci);
                steps += 1;
            }
            total += steps;
        }
    }
    total
}

/// Counts words made up on the spot: strings, hashing and a map, most of it library code.
fn words() -> u64 {
    let mut random = Lcg(3);
    let mut counts: HashMap<String, u32> = HashMap::new();
    for _ in 0..1_500_000 {
        let length = 1 + random.next() % 3;
        let word: String = (0..length).map(|_| (b'a' + (random.next() % 8) as u8) as char).collect();
        *counts.entry(word).or_default() += 1;
    }
    let mut pairs: Vec<(&String, &u32)> = counts.iter().collect();
    pairs.sort();
    pairs.iter().fold(0u64, |sum, (word, count)| {
        sum.wrapping_mul(131).wrapping_add(word.len() as u64 * 7 + u64::from(**count))
    })
}
