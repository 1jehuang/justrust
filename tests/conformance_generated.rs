//! Bounded cross-feature differential probes, not exhaustive Rust conformance.
//! Reproduce a case by its test name and the fixed seed printed with its source.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

const SEED: u64 = 0x6a09_e667_f3bc_c909;
const VALID_CASES: usize = 12;
const INVALID_CASES: usize = 8;

// Each ID has its own state: adding or reordering tests cannot perturb others.
// Arithmetic operands stay in 1..=13, and each source has at most two closures.
fn operands(id: usize) -> (i32, i32) {
    let mut state = SEED ^ (id as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    ((state % 13 + 1) as i32, ((state >> 16) % 13 + 1) as i32)
}

fn valid(id: usize) -> (String, String) {
    assert!(id < VALID_CASES);
    let (a, b) = operands(id);
    match id % 4 {
        0 => (
            format!(
                "fn identity<T>(x:T)->T{{x}} fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{{f(x)}} fn main(){{let pair=identity(({a}i32,{b}i32));let r=&pair.0;let f=|x:i32|{{let g=|y:i32|y+1;g(x)+*r}};let result=Some(apply(f,pair.1));match result{{Some(n)=>println!(\"{{}}\",n),None=>println!(\"none\")}}}}"
            ),
            format!("{}\n", a + b + 1),
        ),
        1 => (
            format!(
                "fn identity<T>(x:T)->T{{x}} fn apply<F:FnMut(i32)->i32>(mut f:F,x:i32)->i32{{f(x)}} fn main(){{let pair=identity(({a}i32,{b}i32));let mut n=pair.0;let r=&mut n;let f=|x:i32|{{*r+=x;*r}};let result=Some(apply(f,pair.1));match result{{Some(value)=>println!(\"{{}}\",value),None=>println!(\"none\")}}println!(\"{{}}\",n);}}"
            ),
            format!("{0}\n{0}\n", a + b),
        ),
        2 => (
            format!(
                "struct Guard{{n:i32}} impl Drop for Guard{{fn drop(&mut self){{println!(\"drop {{}}\",self.n);}}}} fn identity<T>(x:T)->T{{x}} fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{{f(x)}} fn main(){{let guard=Guard{{n:{a}}};let pair=identity(({a}i32,{b}i32));let f=|x:i32|{{let g=|y:i32|y+1;g(x)+guard.n}};match Some(apply(f,pair.1)){{Some(n)=>println!(\"{{}}\",n),None=>println!(\"none\")}}}}"
            ),
            format!("{}\ndrop {a}\n", a + b + 1),
        ),
        3 => (
            format!(
                "fn identity<T>(x:T)->T{{x}} fn apply<F:FnOnce()->i32>(f:F)->i32{{f()}} fn main(){{let pair=identity(({a}i32,{b}i32));let r=&pair.1;let f=||{{let g=|x:i32|x+*r;g(pair.0)}};match Some(apply(f)){{Some(n)=>println!(\"{{}}\",n),None=>println!(\"none\")}}}}"
            ),
            format!("{}\n", a + b),
        ),
        _ => unreachable!(),
    }
}

fn invalid(id: usize) -> (String, &'static str) {
    assert!(id < INVALID_CASES);
    let (a, b) = operands(VALID_CASES + id);
    // Every defect is in an uncalled function. Main alone is deliberately valid.
    let (body, code) = match id {
        0 => (
            format!(
                "fn unused(){{let pair=({a}i32,{b}i32);let f=||->bool{{pair.0}};let value=Some(f());}}"
            ),
            "E0308",
        ),
        1 => (
            format!(
                "fn unused(){{let pair=({a}i32,{b}i32);let mut n=pair.0;let f=||{{n+=pair.1;}};f();}}"
            ),
            "E0596",
        ),
        2 => (
            format!(
                "fn unused(){{let pair=({a}i32,{b}i32);let mut n=pair.0;let f=||n+pair.1;n=0;let result=Some(f());}}"
            ),
            "E0506",
        ),
        3 => (
            format!(
                "fn unused(){{let pair=({a}i32,{b}i32);let mut n=pair.0;let mut f=||{{n+=pair.1;n}};let value=n;let result=Some(f());}}"
            ),
            "E0503",
        ),
        4 => (
            format!(
                "fn apply<F:Fn()->i32>(f:F)->i32{{f()}} fn unused(){{let pair=({a}i32,{b}i32);let mut n=pair.0;let f=||{{n+=pair.1;n}};let result=Some(apply(f));}}"
            ),
            "E0525",
        ),
        5 => (
            format!(
                "struct Item{{n:i32}} fn identity<T>(x:T)->T{{x}} fn unused(){{let item=identity(Item{{n:{a}}});let f=move||item;let result=Some(f());let second=f();}}"
            ),
            "E0382",
        ),
        6 => (
            format!(
                "fn requires_copy<T:Copy>(x:T){{}} fn unused<T>(value:T){{let pair=({a}i32,{b}i32);let f=move||{{requires_copy(value);pair.0}};let result=Some(f());}}"
            ),
            "E0277",
        ),
        7 => (
            format!(
                "fn identity<T>(x:T)->T{{x}} fn unused(){{let r;{{let pair=identity(({a}i32,{b}i32));r=&pair.0;}}let f=||*r;let result=Some(f());}}"
            ),
            "E0597",
        ),
        _ => unreachable!(),
    };
    (format!("{body} fn main(){{}}"), code)
}

fn probe(id: usize, is_invalid: bool) {
    let kind = if is_invalid { "invalid" } else { "valid" };
    let tag = format!("generated_seed_{SEED:016x}_{kind}_{id:02}");
    let (source, expectation) = if is_invalid {
        let (source, code) = invalid(id);
        (source, Expect::Invalid(code))
    } else {
        let (source, stdout) = valid(id);
        (source, Expect::Run(Box::leak(stdout.into_boxed_str())))
    };
    // The oracle currently takes static strings. These bounded process-lifetime
    // allocations retain exact reproduction text even if the oracle panics.
    let source: &'static str = Box::leak(source.into_boxed_str());
    let name: &'static str = Box::leak(format!("{tag}\nsource:\n{source}").into_boxed_str());
    let outcome = std::panic::catch_unwind(|| {
        oracle::check(
            &tag,
            &[Case {
                name,
                source,
                expect: expectation,
            }],
        );
    });
    if let Err(error) = outcome {
        let detail = error
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| error.downcast_ref::<&str>().copied())
            .unwrap_or("non-string oracle panic");
        panic!("{name}\noracle failure: {detail}");
    }
}

// Independent tests ensure one failing combination does not suppress the rest.
macro_rules! probes {
    ($invalid:literal; $($name:ident = $id:literal),+ $(,)?) => {
        $(#[test] fn $name() { probe($id, $invalid); })+
    };
}
probes!(false;
    valid_00 = 0, valid_01 = 1, valid_02 = 2, valid_03 = 3,
    valid_04 = 4, valid_05 = 5, valid_06 = 6, valid_07 = 7,
    valid_08 = 8, valid_09 = 9, valid_10 = 10, valid_11 = 11,
);
probes!(true;
    unused_invalid_00 = 0, unused_invalid_01 = 1,
    unused_invalid_02 = 2, unused_invalid_03 = 3,
    unused_invalid_04 = 4, unused_invalid_05 = 5,
    unused_invalid_06 = 6, unused_invalid_07 = 7,
);
