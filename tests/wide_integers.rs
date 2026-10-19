//! Frozen executable stock oracle for the next native i128/u128 implementation.
//! `stock_oracle` never invokes the experimental compiler. The native gate first
//! reruns the entire independent oracle, then requires CHECK/BUILD at workers 1/8.
//! Source arithmetic requires SIGABRT; indexed compound guards retain exit 101.
//! Core arithmetic cases do not require shifts, binary bitwise operators, compound
//! assignments, or new cast syntax. Separate mixed-width shift regressions cover
//! existing shift syntax with a newly wide RHS. All cases target native acceptance.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
const IMAX: &str = "170141183460469231731687303715884105727";
const IMIN: &str = "-170141183460469231731687303715884105728";
const UMAX: &str = "340282366920938463463374607431768211455";

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for _ in 0..100 {
            let path = base.join(format!(
                "jr-wide-integers-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("scratch: {e}"),
            }
        }
        panic!("scratch allocation exhausted")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn file(path: &Path) -> fs::File {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap()
}
fn capture(command: &mut Command, scratch: &Scratch) -> (i32, Vec<u8>, Vec<u8>) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    let mut child = Running(
        command
            .stdin(Stdio::null())
            .stdout(file(&out))
            .stderr(file(&err))
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {command:?}: {e}")),
    );
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= 65536,
                "capture exceeded 64KiB: {command:?}"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(fs::metadata(path).unwrap().len() <= 65536);
    }
    (
        status
            .code()
            .unwrap_or_else(|| -status.signal().unwrap_or(0)),
        fs::read(out).unwrap(),
        fs::read(err).unwrap(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Valid,
    Trap,
    Invalid,
    // Legal Rust outside the current numeric-cast checkpoint, not invalid Rust.
    Unsupported,
}
struct Fixture {
    name: String,
    source: String,
    outcome: Outcome,
    diagnostic: &'static str,
}
fn valid(name: &str, definitions: &str, checks: &str) -> Fixture {
    Fixture {
        name: name.into(),
        source: format!(
            "{definitions}\nfn run()->bool{{{checks}\ntrue}}\nfn main(){{if run(){{println!(\"wide ok\");}}}}"
        ),
        outcome: Outcome::Valid,
        diagnostic: "",
    }
}
fn invalid(name: &str, body: &str, diagnostic: &'static str) -> Fixture {
    Fixture {
        name: name.into(),
        source: format!("fn main(){{{body}}}"),
        outcome: Outcome::Invalid,
        diagnostic,
    }
}
fn trap(name: &str, ty: &str, expression: &str, arguments: &str) -> Fixture {
    // Runtime arguments prevent rustc's compile-time overflow/unconditional-panic
    // diagnostics from being confused with the native runtime checked policy.
    Fixture {
        name: name.into(),
        source: format!(
            "fn calculate(a:{ty},b:{ty})->{ty}{{{expression}}}\nfn main(){{println!(\"before trap\");let result=calculate({arguments});if result==0{ty}{{println!(\"after trap\");}}}}"
        ),
        outcome: Outcome::Trap,
        diagnostic: "",
    }
}
fn fixtures() -> Vec<Fixture> {
    let mut cases = vec![
        valid(
            "signed_literal_limits",
            "",
            &format!(
                "let lo={IMIN}i128;let hi={IMAX}i128;if lo>=0i128 || hi<=0i128 || lo+hi != -1i128{{return false;}}"
            ),
        ),
        valid(
            "unsigned_literal_limits",
            "",
            &format!(
                "let lo=0u128;let hi={UMAX}u128;if lo>=hi || hi-1u128 != 340282366920938463463374607431768211454u128{{return false;}}"
            ),
        ),
        valid(
            "hexadecimal_literals",
            "",
            &format!(
                "let u=0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffffu128;let s=0x7fff_ffff_ffff_ffff_ffff_ffff_ffff_ffffi128;if u!={UMAX}u128 || s!={IMAX}i128{{return false;}}"
            ),
        ),
        valid(
            "binary_literals",
            "",
            &format!(
                "let u=0b{}u128;let s=0b{}i128;if u!={UMAX}u128 || s!={IMAX}i128{{return false;}}",
                "1".repeat(128),
                "1".repeat(127)
            ),
        ),
        valid(
            "octal_literals",
            "",
            &format!(
                "let u=0o3{}u128;let s=0o1{}i128;if u!={UMAX}u128 || s!={IMAX}i128{{return false;}}",
                "7".repeat(42),
                "7".repeat(42)
            ),
        ),
        valid(
            "return_literal_context",
            &format!("fn signed()->i128{{{IMIN}}}fn unsigned()->u128{{{UMAX}}}"),
            &format!("if signed()!={IMIN}i128 || unsigned()!={UMAX}u128{{return false;}}"),
        ),
        valid(
            "binding_call_array_context",
            "fn accept(x:u128)->u128{x}",
            &format!(
                "let x:i128={IMAX};let y:u128={UMAX};let a:[u128;2]=[0,{UMAX}];if x!={IMAX}i128 || accept({UMAX})!=y || a[1usize]!=y{{return false;}}"
            ),
        ),
        valid(
            "minimum_literal_forms",
            "",
            &format!(
                "let a:i128={IMIN};let b=-0x8000_0000_0000_0000_0000_0000_0000_0000i128;let c={IMIN}i128;if a!=b || b!=c || -0i128!=0i128{{return false;}}"
            ),
        ),
        valid(
            "signed_high_arithmetic",
            "fn calc(a:i128,b:i128)->i128{(a+b-b)*b/b}",
            "let a=18446744073709551617i128;let b=3i128;if calc(a,b)!=a || -a+a!=0i128 || a-a!=0i128{return false;}if a+b!=18446744073709551620i128 || a-b!=18446744073709551614i128 || a*b!=55340232221128654851i128 || (-a)*b!=-55340232221128654851i128{return false;}let limb=4294967296i128;if limb*limb!=18446744073709551616i128{return false;}let n=-a;if n-b != -18446744073709551620i128{return false;}",
        ),
        valid(
            "unsigned_high_arithmetic",
            "fn calc(a:u128,b:u128)->u128{(a+b-b)*b/b}",
            "let a=18446744073709551617u128;let b=3u128;if calc(a,b)!=a || a-a!=0u128 || a+b!=18446744073709551620u128 || a-b!=18446744073709551614u128 || a*b!=55340232221128654851u128{return false;}let limb=4294967296u128;if limb*limb!=18446744073709551616u128{return false;}",
        ),
        valid(
            "signed_div_rem_quadrants",
            "fn q(a:i128,b:i128)->i128{a/b}fn r(a:i128,b:i128)->i128{a%b}",
            "let a=18446744073709551619i128;if q(a,3i128)!=6148914691236517206i128 || r(a,3i128)!=1i128{return false;}if q(-a,3i128)!=-6148914691236517206i128 || r(-a,3i128)!=-1i128{return false;}if q(a,-3i128)!=-6148914691236517206i128 || r(a,-3i128)!=1i128{return false;}if q(-a,-3i128)!=6148914691236517206i128 || r(-a,-3i128)!=-1i128{return false;}",
        ),
        valid(
            "unsigned_high_div_rem",
            "fn q(a:u128,b:u128)->u128{a/b}fn r(a:u128,b:u128)->u128{a%b}",
            &format!(
                "let a={UMAX}u128;let b=18446744073709551616u128;if q(a,b)!=18446744073709551615u128 || r(a,b)!=18446744073709551615u128{{return false;}}if q(a,a)!=1u128 || r(a,a)!=0u128{{return false;}}"
            ),
        ),
        valid(
            "signed_comparisons",
            "",
            &format!(
                "let a={IMIN}i128;let b={IMAX}i128;let c=-18446744073709551616i128;let d=-18446744073709551615i128;if !(a<c) || !(c<d) || !(d<b) || !(b>a) || !(a<=a) || !(b>=b) || a==b || !(a!=b){{return false;}}"
            ),
        ),
        valid(
            "unsigned_comparisons",
            "",
            &format!(
                "let a=9223372036854775808u128;let b=18446744073709551616u128;let c={UMAX}u128;if !(a<b) || !(b<c) || !(c>a) || !(a<=a) || !(c>=c) || a==b || !(a!=b){{return false;}}"
            ),
        ),
        valid(
            "signed_not_and_neg",
            "",
            &format!(
                "let hi={IMAX}i128;let lo={IMIN}i128;let a=18446744073709551617i128;if !hi!=lo || !lo!=hi || !0i128 != -1i128 || !!a!=a || -(-a)!=a{{return false;}}"
            ),
        ),
        valid(
            "unsigned_not",
            "",
            &format!(
                "let hi={UMAX}u128;let a=18446744073709551617u128;if !hi!=0u128 || !0u128!=hi || !!a!=a || !a!=340282366920938463444927863358058659838u128{{return false;}}"
            ),
        ),
    ];

    // Each <=64-bit source/destination gets its own executable fixture. The
    // signed -1 and unsigned maximum distinguish sign extension from zero extension.
    for (ty, min, max) in [
        ("i8", "-128", "127"),
        ("i16", "-32768", "32767"),
        ("i32", "-2147483648", "2147483647"),
        ("i64", "-9223372036854775808", "9223372036854775807"),
        ("isize", "-9223372036854775808", "9223372036854775807"),
        ("u8", "0", "255"),
        ("u16", "0", "65535"),
        ("u32", "0", "4294967295"),
        ("u64", "0", "18446744073709551615"),
        ("usize", "0", "18446744073709551615"),
    ] {
        let signed = ty.starts_with('i');
        let extra = if signed {
            format!(
                "let n=-1{ty};if (n as i128)!=-1i128 || (n as u128)!={UMAX}u128{{return false;}}"
            )
        } else {
            String::new()
        };
        cases.push(valid(&format!("cast_{ty}_to_128"), "", &format!(
            "let lo={min}{ty};let hi={max}{ty};if (lo as i128)!={min}i128 || (hi as i128)!={max}i128 || (hi as u128)!={max}u128{{return false;}}{extra}"
        )));
        let minus_one = if signed { "-1" } else { max };
        cases.push(valid(&format!("cast_128_to_{ty}"), "", &format!(
            "let all={UMAX}u128;let neg=-1i128;let high=18446744073709551616u128;if (all as {ty})!={minus_one}{ty} || (neg as {ty})!={minus_one}{ty} || (high as {ty})!=0{ty} || (257u128 as {ty})!=(257u128 as u64 as {ty}){{return false;}}let lo={min}i128;let hi={max}u128;if (lo as {ty})!={min}{ty} || (hi as {ty})!={max}{ty}{{return false;}}"
        )));
    }
    cases.extend([
        valid("cast_signed_unsigned_128", "", &format!("let a=-1i128;let b={UMAX}u128;let c={IMIN}i128;if (a as u128)!=b || (b as i128)!=a || (c as u128)!=170141183460469231731687303715884105728u128 || (170141183460469231731687303715884105728u128 as i128)!=c{{return false;}}")),
    ]);
    for float in ["f32", "f64"] {
        cases.push(valid(&format!("{float}_to_128_special"), "", &format!(
            "let z=0.0{float};let nan=z/z;let inf=1.0{float}/z;if (nan as i128)!=0i128 || (nan as u128)!=0u128 || (inf as i128)!={IMAX}i128 || (-inf as i128)!={IMIN}i128 || (inf as u128)!={UMAX}u128 || (-inf as u128)!=0u128 || (-0.0{float} as i128)!=0i128 || (-0.0{float} as u128)!=0u128{{return false;}}"
        )));
        cases.push(valid(&format!("{float}_to_128_finite"), "", &format!(
            "if (12.75{float} as i128)!=12i128 || (-12.75{float} as i128)!=-12i128 || (12.75{float} as u128)!=12u128 || (-12.75{float} as u128)!=0u128{{return false;}}if (170141183460469231731687303715884105728.0{float} as i128)!={IMAX}i128 || (-170141183460469231731687303715884105728.0{float} as i128)!={IMIN}i128 || (170141183460469231731687303715884105728.0{float} as u128)!=170141183460469231731687303715884105728u128{{return false;}}"
        )));
        let rounding = if float == "f32" {
            format!(
                "let inf=1.0f32/0.0f32;if ({UMAX}u128 as f32)!=inf || (16777217u128 as f32)!=16777216.0f32 || (16777219i128 as f32)!=16777220.0f32{{return false;}}let sticky=1267650675786093127411026624513u128;if (sticky as f32)!=1267650751343956853325350043648.0f32 || (-1267650675786093127411026624513i128 as f32)!=-1267650751343956853325350043648.0f32{{return false;}}"
            )
        } else {
            format!(
                "if ({UMAX}u128 as f64)!=340282366920938463463374607431768211456.0f64 || (9007199254740993u128 as f64)!=9007199254740992.0f64 || (9007199254740995i128 as f64)!=9007199254740996.0f64{{return false;}}if (340282366920938463463374607431768211456.0f64 as u128)!={UMAX}u128 || (340282366920938425684442744474606501888.0f64 as u128)!=340282366920938425684442744474606501888u128{{return false;}}"
            )
        };
        cases.push(valid(&format!("128_to_{float}_rounding"), "", &format!(
            "if (18446744073709551616u128 as {float})!=18446744073709551616.0{float} || ({IMIN}i128 as {float})!=-170141183460469231731687303715884105728.0{float} || ({IMAX}i128 as {float})!=170141183460469231731687303715884105728.0{float}{{return false;}}{rounding}"
        )));
    }
    cases.extend([
        valid("generic_identity", "fn identity<T>(x:T)->T{x}", &format!("let a=identity({IMIN}i128);let b=identity::<u128>({UMAX}u128);if a!={IMIN}i128 || b!={UMAX}u128{{return false;}}")),
        valid("generic_copy", "fn twice<T:Copy>(x:T)->(T,T){(x,x)}", &format!("let a=twice::<i128>({IMIN}i128);let b=twice({UMAX}u128);if a.0!=a.1 || b.0!=b.1 || a.0!={IMIN}i128 || b.0!={UMAX}u128{{return false;}}")),
        valid("generic_clone", "fn duplicate<T:Clone>(x:&T)->T{<T as Clone>::clone(x)}", &format!("let a={IMIN}i128;let b={UMAX}u128;let c=duplicate::<i128>(&a);let d=<u128 as Clone>::clone(&b);let p=(a,b);let q=<(i128,u128) as Clone>::clone(&p);if c!=a || d!=b || q.0!=a || q.1!=b{{return false;}}")),
        valid("shared_reference_return", "fn keep(x:&i128)->&i128{x}fn generic<T>(x:&T)->&T{x}", &format!("let a={IMIN}i128;let b={UMAX}u128;let r=keep(&a);let s=generic::<u128>(&b);if *r!=a || *s!=b{{return false;}}")),
        valid("mutable_reference_return", "fn keep(x:&mut u128)->&mut u128{x}", &format!("let mut a={UMAX}u128;let r=keep(&mut a);*r=*r-18446744073709551616u128;if a!=340282366920938463444927863358058659839u128{{return false;}}let mut b={IMIN}i128;let s=&mut b;*s=*s+1i128;if b!=-170141183460469231731687303715884105727i128{{return false;}}")),
        valid("mixed_tuple_layout", "fn pass(x:(u8,i128,bool,u128,i64))->(u8,i128,bool,u128,i64){x}", &format!("let a=pass((7u8,{IMIN}i128,true,{UMAX}u128,-9i64));let b=a;if a.0!=7u8 || b.1!={IMIN}i128 || !a.2 || b.3!={UMAX}u128 || a.4!=-9i64{{return false;}}")),
        valid("mixed_array_layout", "fn pass(x:[(u8,i128,u128);2])->[(u8,i128,u128);2]{x}", &format!("let a=pass([(1u8,{IMIN}i128,{UMAX}u128),(2u8,{IMAX}i128,18446744073709551616u128)]);let b=a;if b[0usize].0!=1u8 || a[0usize].1!={IMIN}i128 || b[0usize].2!={UMAX}u128 || a[1usize].1!={IMAX}i128 || b[1usize].2!=18446744073709551616u128{{return false;}}")),
        valid("mixed_enum_payload", "enum E{A(u8,i128,u128),B{small:bool,wide:u128}}fn pass(x:E)->E{x}", &format!("let a=pass(E::A(7u8,{IMIN}i128,{UMAX}u128));match a{{E::A(s,i,u)=>{{if s!=7u8 || i!={IMIN}i128 || u!={UMAX}u128{{return false;}}}},E::B{{small:s,wide:u}}=>{{return false;}}}}let b=pass(E::B{{small:true,wide:{UMAX}u128}});match b{{E::B{{small:s,wide:u}}=>{{if !s || u!={UMAX}u128{{return false;}}}},E::A(s,i,u)=>{{return false;}}}}")),
        valid("mixed_struct_and_returns", "struct S{tag:u8,signed:i128,flag:bool,unsigned:u128}fn signed(x:i128)->i128{x}fn unsigned(x:u128)->u128{x}fn pass(x:S)->S{x}", &format!("let s=pass(S{{tag:3u8,signed:signed({IMIN}i128),flag:true,unsigned:unsigned({UMAX}u128)}});if s.tag!=3u8 || s.signed!={IMIN}i128 || !s.flag || s.unsigned!={UMAX}u128{{return false;}}")),
    ]);
    for (name, ty, expression, arguments) in [
        (
            "signed_add_overflow",
            "i128",
            "a+b",
            format!("{IMAX}i128,1i128"),
        ),
        (
            "signed_sub_overflow",
            "i128",
            "a-b",
            format!("{IMIN}i128,1i128"),
        ),
        (
            "signed_mul_overflow",
            "i128",
            "a*b",
            format!("{IMAX}i128,2i128"),
        ),
        (
            "signed_div_overflow",
            "i128",
            "a/b",
            format!("{IMIN}i128,-1i128"),
        ),
        (
            "signed_rem_overflow",
            "i128",
            "a%b",
            format!("{IMIN}i128,-1i128"),
        ),
        (
            "signed_neg_overflow",
            "i128",
            "-a",
            format!("{IMIN}i128,0i128"),
        ),
        (
            "unsigned_add_overflow",
            "u128",
            "a+b",
            format!("{UMAX}u128,1u128"),
        ),
        ("unsigned_sub_overflow", "u128", "a-b", "0u128,1u128".into()),
        (
            "unsigned_mul_overflow",
            "u128",
            "a*b",
            format!("{UMAX}u128,2u128"),
        ),
        (
            "signed_div_zero",
            "i128",
            "a/b",
            "18446744073709551617i128,0i128".into(),
        ),
        (
            "signed_rem_zero",
            "i128",
            "a%b",
            "18446744073709551617i128,0i128".into(),
        ),
        (
            "unsigned_div_zero",
            "u128",
            "a/b",
            "18446744073709551617u128,0u128".into(),
        ),
        (
            "unsigned_rem_zero",
            "u128",
            "a%b",
            "18446744073709551617u128,0u128".into(),
        ),
    ] {
        cases.push(trap(name, ty, expression, &arguments));
    }
    cases.extend([
        invalid(
            "oversized_unsigned_decimal",
            "let x=340282366920938463463374607431768211456u128;",
            "literal",
        ),
        invalid(
            "oversized_signed_decimal",
            "let x=170141183460469231731687303715884105728i128;",
            "literal",
        ),
        invalid(
            "below_signed_minimum",
            "let x=-170141183460469231731687303715884105729i128;",
            "literal",
        ),
        invalid(
            "oversized_unsigned_hex",
            "let x=0x1_0000_0000_0000_0000_0000_0000_0000_0000u128;",
            "literal",
        ),
        invalid(
            "oversized_signed_hex",
            "let x=0x8000_0000_0000_0000_0000_0000_0000_0000i128;",
            "literal",
        ),
        invalid(
            "oversized_unsigned_binary",
            &format!("let x=0b1{}u128;", "0".repeat(128)),
            "literal",
        ),
        invalid(
            "oversized_unsigned_octal",
            &format!("let x=0o4{}u128;", "0".repeat(42)),
            "literal",
        ),
        invalid("neg_unsigned", "let x=1u128;let y=-x;", "E0600"),
        invalid("neg_unsigned_zero", "let x=-0u128;", "E0600"),
        invalid(
            "mixed_arithmetic",
            "let x=1i128;let y=1u128;let z=x+y;",
            "E0308",
        ),
        invalid(
            "mixed_comparison",
            "let x=1i128;let y=1u128;let z=x==y;",
            "E0308",
        ),
        invalid("mixed_assignment", "let mut x=1i128;x=1u128;", "E0308"),
        Fixture {
            name: "mixed_return".into(),
            source: "fn bad()->i128{1u128}fn main(){let x=bad();}".into(),
            outcome: Outcome::Invalid,
            diagnostic: "E0308",
        },
    ]);
    // Integration regressions requested after the initial 79-source stock freeze:
    // RHS parameters keep count validation at runtime, where width must be the LHS.
    cases.extend([
        valid(
            "narrow_shift_wide_rhs",
            "fn left(x:u8,n:u128)->u8{x<<n}fn right(x:i8,n:i128)->i8{x>>n}",
            "if left(1u8,7u128)!=128u8 || left(255u8,1u128)!=254u8 || right(-128i8,7i128)!=-1i8 || right(-64i8,2i128)!=-16i8{return false;}",
        ),
        valid(
            "indexed_compound_shift_wide_rhs",
            "fn left(a:&mut [u8;2],index:usize,n:u128){a[index]<<=n;}fn right(a:&mut [i16;2],index:usize,n:i128){a[index]>>=n;}",
            "let mut a=[3u8,129u8];left(&mut a,1usize,1u128);let mut b=[7i16,-32768i16];right(&mut b,1usize,15i128);if a[0usize]!=3u8 || a[1usize]!=2u8 || b[0usize]!=7i16 || b[1usize]!=-1i16{return false;}",
        ),
    ]);
    for (name, definitions, body) in [
        (
            "narrow_left_shift_at_width",
            "fn shift(x:u8,n:u128)->u8{x<<n}",
            "let x=shift(1u8,8u128);if x==0u8{println!(\"after trap\");}",
        ),
        (
            "narrow_signed_right_shift_at_width",
            "fn shift(x:i8,n:i128)->i8{x>>n}",
            "let x=shift(-128i8,8i128);if x==0i8{println!(\"after trap\");}",
        ),
        (
            "indexed_compound_shift_at_width",
            "fn shift(a:&mut [u8;2],i:usize,n:u128){a[i]<<=n;}",
            "let mut a=[1u8,2u8];shift(&mut a,1usize,8u128);if a[1usize]==0u8{println!(\"after trap\");}",
        ),
        (
            "narrow_shift_negative_wide_count",
            "fn shift(x:i8,n:i128)->i8{x>>n}",
            "let x=shift(-128i8,-1i128);if x==0i8{println!(\"after trap\");}",
        ),
        (
            "narrow_shift_high_limb_count",
            "fn shift(x:u8,n:u128)->u8{x<<n}",
            "let x=shift(1u8,18446744073709551616u128);if x==0u8{println!(\"after trap\");}",
        ),
    ] {
        cases.push(Fixture {
            name: name.into(),
            source: format!("{definitions}\nfn main(){{println!(\"before trap\");{body}}}"),
            outcome: Outcome::Trap,
            diagnostic: "",
        });
    }
    let mut bool_cast = valid(
        "legal_unsupported_bool_cast",
        "",
        "let yes=true;let no=false;if (yes as i128)!=1i128 || (no as i128)!=0i128 || (yes as u128)!=1u128 || (no as u128)!=0u128{return false;}",
    );
    bool_cast.outcome = Outcome::Unsupported;
    cases.push(bool_cast);
    cases.push(Fixture {
        name: "historical_integer_128_types".into(),
        // Exact source promoted from integer_types.rs, without wrapping/reformatting.
        source: r#"fn id(x:i128)->i128{x} fn run()->i32{let x=id(-170141183460469231731687303715884105728i128);let y:u128=340282366920938463463374607431768211455u128;if x<0i128 && y>0u128{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#.into(),
        outcome: Outcome::Valid,
        diagnostic: "",
    });
    // Source observed independently on hosted stock Rust before any native gate.
    cases.push(Fixture {
        name: "index_rebinds_current_carrier".into(),
        source: r#"fn main(){let mut a:[u128;1]=[10u128];let mut b:[u128;1]=[20u128];let mut r=&mut a;r[{r=&mut b;0usize}]+=1u128;if a[0usize]==10u128{println!("a=10");}if a[0usize]==11u128{println!("a=11");}if b[0usize]==20u128{println!("b=20");}if b[0usize]==21u128{println!("b=21");}}"#.into(),
        outcome: Outcome::Valid,
        diagnostic: "",
    });
    for (name, source) in [
        (
            "historical_integer128_bitwise",
            r#"fn run()->i32{let x=1u128|2u128;let y=-1i128&7i128;if x==3u128 && y==7i128{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        ),
        (
            "historical_integer128_cast",
            r#"fn run()->i32{let x=300u128 as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        ),
        (
            "historical_integer128_compounds",
            r#"fn run()->i32{let mut x=3u128;x+=4u128;x<<=2u8;x>>=2u8;if x==7u128{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        ),
        (
            "historical_integer128_shifts",
            r#"fn run()->i32{let x=1u128<<100u8;let y=x>>100u8;if y==1u128{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        ),
    ] {
        cases.push(Fixture {
            name: name.into(),
            source: source.into(),
            outcome: Outcome::Valid,
            diagnostic: "",
        });
    }
    // Freeze classifications and prevent accidental duplicate/oversized sources.
    let counts = (
        cases.iter().filter(|f| f.outcome == Outcome::Valid).count(),
        cases.iter().filter(|f| f.outcome == Outcome::Trap).count(),
        cases
            .iter()
            .filter(|f| f.outcome == Outcome::Invalid)
            .count(),
        cases
            .iter()
            .filter(|f| f.outcome == Outcome::Unsupported)
            .count(),
    );
    assert_eq!(
        counts,
        (60, 18, 13, 1),
        "frozen source classifications changed"
    );
    let mut names = BTreeSet::new();
    for f in &cases {
        assert!(names.insert(&f.name), "duplicate fixture {}", f.name);
        assert!(f.source.len() <= 8192, "unbounded fixture {}", f.name);
    }
    cases
}
fn expected_stdout(f: &Fixture) -> &'static [u8] {
    if f.name == "historical_integer_128_types" {
        return b"integer types oracle ok\n";
    }
    match f.name.as_str() {
        "index_rebinds_current_carrier" => return b"a=10\nb=21\n",
        "historical_integer128_bitwise" => return b"bitwise oracle ok\n",
        "historical_integer128_cast" => return b"cast oracle ok\n",
        "historical_integer128_compounds" => return b"compound oracle ok\n",
        "historical_integer128_shifts" => return b"shift oracle ok\n",
        _ => {}
    }
    match f.outcome {
        Outcome::Valid | Outcome::Unsupported => b"wide ok\n",
        Outcome::Trap => b"before trap\n",
        Outcome::Invalid => b"",
    }
}
fn is_elf(bytes: &[u8]) -> bool {
    bytes.len() >= 64 && &bytes[..6] == b"\x7fELF\x02\x01"
}
fn stock(f: &Fixture) {
    let s = Scratch::new();
    let input = s.path("input.rs");
    let output = s.path("stock");
    file(&input).write_all(f.source.as_bytes()).unwrap();
    let result = capture(
        Command::new("rustc")
            .arg("--edition=2021")
            .args(["-C", "overflow-checks=yes", "-C", "opt-level=0"])
            .arg(&input)
            .arg("-o")
            .arg(&output),
        &s,
    );
    assert_eq!(
        result.0,
        if f.outcome == Outcome::Invalid { 1 } else { 0 },
        "stock {}: {}\n{}",
        f.name,
        String::from_utf8_lossy(&result.2),
        f.source
    );
    assert!(result.1.is_empty(), "stock compiler stdout {}", f.name);
    if f.outcome == Outcome::Invalid {
        let diagnostic = String::from_utf8_lossy(&result.2);
        assert!(
            !diagnostic.is_empty() && diagnostic.contains(f.diagnostic),
            "stock {} missing {}: {diagnostic}",
            f.name,
            f.diagnostic
        );
        assert!(!output.exists(), "invalid stock output {}", f.name);
    } else {
        assert!(is_elf(&fs::read(&output).unwrap()), "stock ELF {}", f.name);
        let run = capture(&mut Command::new(&output), &s);
        let expected = if f.outcome == Outcome::Trap { 101 } else { 0 };
        assert_eq!(
            run.0, expected,
            "stock runtime {}: {run:?}\n{}",
            f.name, f.source
        );
        assert_eq!(run.1, expected_stdout(f), "stock stdout {}", f.name);
        if f.outcome != Outcome::Trap {
            assert!(run.2.is_empty(), "stock runtime stderr {}: {run:?}", f.name);
        } else {
            assert!(
                !run.2.is_empty(),
                "stock trap lacks panic diagnostic {}",
                f.name
            );
        }
    }
    eprintln!("stock wide PASS {:?} {}", f.outcome, f.name);
}
fn oracle_all(cases: &[Fixture]) {
    let s = Scratch::new();
    let version = capture(Command::new("rustc").arg("--version"), &s);
    assert_eq!(version.0, 0);
    eprintln!(
        "wide stock oracle: {}",
        String::from_utf8_lossy(&version.1).trim()
    );
    for f in cases {
        stock(f);
    }
    eprintln!(
        "wide frozen sources: 92 = 60 valid + 18 runtime traps + 13 invalid + 1 legal unsupported"
    );
}
#[test]
fn stock_oracle() {
    oracle_all(&fixtures());
}
#[test]
fn native_after_complete_stock_oracle() {
    let cases = fixtures();
    // Do not invoke native until every source has an independent executable or
    // rejecting stock oracle. This gate intentionally fails before native128 lands.
    oracle_all(&cases);
    let mut failures = Vec::new();
    let mut passed = 0;
    for f in &cases {
        for workers in [1, 8] {
            for check in [true, false] {
                let s = Scratch::new();
                let input = s.path("input.rs");
                let output = s.path("native");
                file(&input).write_all(f.source.as_bytes()).unwrap();
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&s.0)
                    .arg(&input)
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&output);
                }
                let result = capture(&mut command, &s);
                let diagnostic = String::from_utf8_lossy(&result.2);
                let rejects = matches!(f.outcome, Outcome::Invalid | Outcome::Unsupported);
                let expected = if rejects { 1 } else { 0 };
                let mut errors = Vec::new();
                if result.0 != expected {
                    errors.push(format!("exit {} expected {expected}", result.0));
                }
                // Successful CLI compilation may print its normal build report.
                // Every rejection or actual compiler failure must keep stdout empty.
                if (rejects || result.0 != 0) && !result.1.is_empty() {
                    errors.push("failure printed stdout".into());
                }
                if diagnostic.contains("panicked at") {
                    errors.push("compiler panic".into());
                }
                if f.outcome == Outcome::Invalid {
                    if diagnostic.is_empty() {
                        errors.push("missing invalid diagnostic".into());
                    }
                    if diagnostic.contains("unsupported") {
                        errors.push("invalid Rust misclassified unsupported".into());
                    }
                }
                if f.outcome == Outcome::Unsupported
                    && (diagnostic.is_empty() || !diagnostic.contains("unsupported"))
                {
                    errors.push("legal unsupported lacks explicit unsupported diagnostic".into());
                }
                if check || rejects || result.0 != 0 {
                    for path in [&output, &s.path("input"), &s.path("a.out")] {
                        if path.exists() {
                            errors.push(format!("CHECK/failure published {}", path.display()));
                        }
                    }
                } else if output.exists() {
                    if !is_elf(&fs::read(&output).unwrap()) {
                        errors.push("not ELF64 little-endian".into());
                    } else {
                        let run = capture(&mut Command::new(&output), &s);
                        let expected_exit = if f.outcome != Outcome::Trap {
                            0
                        } else if f.name == "indexed_compound_shift_at_width" {
                            101 // Compound-place guards do not yet carry source sites.
                        } else {
                            -6 // Source-backed ordinary arithmetic uses panic=abort.
                        };
                        if run.0 != expected_exit
                            || run.1 != expected_stdout(f)
                            || (f.outcome == Outcome::Valid && !run.2.is_empty())
                        {
                            errors.push(format!(
                                "runtime {run:?}, expected exit {expected_exit} stdout {:?}",
                                expected_stdout(f)
                            ));
                        }
                    }
                } else {
                    errors.push("BUILD missing output".into());
                }
                if errors.is_empty() {
                    passed += 1;
                } else {
                    failures.push(format!(
                        "{} workers={workers} check={check}: {}\n{diagnostic}\n{}",
                        f.name,
                        errors.join(", "),
                        f.source
                    ));
                }
            }
        }
    }
    eprintln!(
        "native wide frozen results: {passed}/{} CHECK/BUILD gates passed, {} failed",
        cases.len() * 4,
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
