//! Native floating-point CLI regressions, checked against executable stock Rust.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("jr-floats-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch: {error}"),
            }
        }
        panic!("could not allocate scratch")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn source(&self, source: &str) -> PathBuf {
        let path = self.path("input.rs");
        fs::write(&path, source).unwrap();
        path
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
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Output {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    let file = |path: &Path| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap()
    };
    command
        .stdin(Stdio::null())
        .stdout(file(&stdout))
        .stderr(file(&stderr));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {command:?}: {e}")),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timed out: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    Output {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}
fn compiler(input: &Path, output: &Path, workers: usize, check: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command.arg(input).args(["--workers", &workers.to_string()]);
    if check {
        command.arg("--check");
    } else {
        command.arg("-o").arg(output);
    }
    command
}
fn success(result: &Output, context: &str) {
    assert!(
        result.status.success(),
        "{context}: {result:?}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn runs(source: &str) {
    let scratch = Scratch::new();
    let input = scratch.source(source);
    let stock = scratch.path("stock");
    // This is an executable oracle, not host constant folding alone. Override for
    // a pinned toolchain without making rustup or network access a prerequisite.
    let rustc = std::env::var_os("JUSTRUST_TEST_RUSTC").unwrap_or_else(|| "rustc".into());
    let mut command = Command::new(rustc);
    command
        .arg(&input)
        .args(["--edition=2024", "-A", "warnings"])
        .arg("-o")
        .arg(&stock);
    success(&capture(&mut command, &scratch, 30), source);
    let oracle = capture(&mut Command::new(&stock), &scratch, 5);
    success(&oracle, source);
    assert_eq!(
        oracle.stdout, b"float oracle ok\n",
        "stock expectation: {source}"
    );
    assert!(oracle.stderr.is_empty());
    for workers in [1, 8] {
        let output = scratch.path(&format!("native-{workers}"));
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            success(
                &result,
                &format!("workers={workers}, check={check}\n{source}"),
            );
            if check {
                assert!(!output.exists(), "check published executable");
            }
        }
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.len() >= 64, "truncated ELF");
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        let result = capture(&mut Command::new(output), &scratch, 5);
        assert_eq!(
            result.status.code(),
            oracle.status.code(),
            "{result:?}\n{source}"
        );
        assert_eq!(result.stdout, oracle.stdout, "workers={workers}\n{source}");
        assert_eq!(result.stderr, oracle.stderr, "workers={workers}\n{source}");
    }
}
fn body(checks: &str) -> String {
    format!(
        "fn run()->bool{{{checks}true}} fn main(){{if run(){{println!(\"float oracle ok\");}}}}"
    )
}
fn rejects(label: &str, source: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: accepted workers={workers}, check={check}\n{source}"
            );
            assert!(
                result.status.code().is_some(),
                "{label}: signal: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "{label}: no diagnostic");
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
                "{label}: panic: {result:?}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

#[test]
fn remainder_extreme_exponent_gaps_infinity_and_signed_zero() {
    for (ty, huge, tiny) in [
        ("f32", "1.2676506002282294e30", "1.401298464324817e-45"),
        ("f64", "1.0715086071862673e301", "4.9406564584124654e-324"),
    ] {
        runs(&body(&format!(
            r#"
            let huge={huge}{ty}; let tiny={tiny}{ty};
            if huge%tiny != 0.0{ty} {{return false;}}
            if huge%3.0{ty} != 1.0{ty} || -huge%3.0{ty} != -1.0{ty} {{return false;}}
            let z=0.0{ty}; let inf=1.0{ty}/z;
            if 7.5{ty}%inf != 7.5{ty} || -7.5{ty}%inf != -7.5{ty} {{return false;}}
            if 7.5{ty}%(-inf) != 7.5{ty} {{return false;}}
            let negative=-4.0{ty}%2.0{ty};
            if negative != z || 1.0{ty}/negative != -inf {{return false;}}
            if 1.0{ty}/(-z%2.0{ty}) != -inf {{return false;}}
            if 1.0{ty}/(4.0{ty}%(-2.0{ty})) != inf {{return false;}}
        "#
        )));
    }
}

#[test]
fn omitted_generic_float_inference_and_canonical_clone() {
    runs(
        r#"
        fn identity<T>(x:T)->T{x}
        fn twice<T:Copy>(x:T)->(T,T){(x,x)}
        fn run()->bool{
            let a=identity(1.25f32); let b=identity(2.5f64);
            let c=twice(3.5f32); let d=identity(4.5);
            if a!=1.25f32 || b!=2.5f64 || c.0+c.1!=7.0f32 || d!=4.5f64{return false;}
            let x=1.5f32; let y=2.5f64;
            let xc=<f32 as Clone>::clone(&x); let yc=<f64 as Clone>::clone(&y);
            let pair=(x,y); let cloned=<(f32,f64) as Clone>::clone(&pair);
            if xc!=x || yc!=y || cloned.0!=x || cloned.1!=y{return false;}
            true
        }
        fn main(){if run(){println!("float oracle ok");}}
    "#,
    );
}

#[test]
fn arithmetic_remainder_and_compound_updates() {
    for ty in ["f32", "f64"] {
        runs(&body(&format!(
            r#"
            let a=7.5{ty}; let b=2.0{ty};
            if a+b != 9.5{ty} {{return false;}}
            if a-b != 5.5{ty} {{return false;}}
            if a*b != 15.0{ty} {{return false;}}
            if a/b != 3.75{ty} {{return false;}}
            if a%b != 1.5{ty} {{return false;}}
            if -a%b != -1.5{ty} {{return false;}}
            if a % -b != 1.5{ty} {{return false;}}
            let mut x=1.5{ty}; x+=2.5{ty}; x-=1.0{ty}; x*=4.0{ty}; x/=2.0{ty}; x%=4.0{ty};
            if x != 2.0{ty} {{return false;}}
            if !(b<a) || !(b<=a) || !(a>b) || !(a>=b) || a==b || !(a!=b) {{return false;}}
        "#
        )));
    }
}

#[test]
fn ieee_zero_nan_infinity_and_subnormal() {
    for (ty, tiny) in [
        ("f32", "1.401298464324817e-45"),
        ("f64", "4.9406564584124654e-324"),
    ] {
        runs(&body(&format!(
            r#"
            let zero=0.0{ty}; let neg=-zero; let inf=1.0{ty}/zero; let ni=1.0{ty}/neg;
            if zero != neg || !(ni < zero) || !(inf > zero) {{return false;}}
            if 1.0{ty}/neg != -inf {{return false;}}
            if inf+1.0{ty} != inf || ni*2.0{ty} != ni {{return false;}}
            if 1.0{ty}/inf != zero {{return false;}}
            let nan=zero/zero;
            if nan==nan || !(nan!=nan) || nan<zero || nan<=zero || nan>zero || nan>=zero {{return false;}}
            if zero==nan || !(zero!=nan) || zero<nan || zero<=nan || zero>nan || zero>=nan {{return false;}}
            let remainder=inf%2.0{ty}; if remainder==remainder {{return false;}}
            let tiny={tiny}{ty};
            if !(tiny>zero) || tiny+tiny != tiny*2.0{ty} || tiny/2.0{ty} != zero {{return false;}}
            if 1.0{ty}/(-tiny/2.0{ty}) != ni {{return false;}}
        "#
        )));
    }
}

#[test]
fn saturating_float_casts_to_every_integer_type() {
    let types = [
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
    ];
    for float in ["f32", "f64"] {
        let mut checks = format!("let z=0.0{float}; let nan=z/z; let inf=1.0{float}/z;");
        for (int, min, max) in types {
            checks.push_str(&format!(
                r#"
                if (nan as {int}) != 0{int} {{return false;}}
                if (inf as {int}) != {max}{int} {{return false;}}
                if (-inf as {int}) != {min}{int} {{return false;}}
                if (1.0e30{float} as {int}) != {max}{int} {{return false;}}
                if (-1.0e30{float} as {int}) != {min}{int} {{return false;}}
                if (12.75{float} as {int}) != 12{int} {{return false;}}
                if (-0.0{float} as {int}) != 0{int} {{return false;}}
            "#
            ));
            let negative = if int.starts_with('u') { "0" } else { "-12" };
            checks.push_str(&format!(
                "if (-12.75{float} as {int}) != {negative}{int} {{return false;}}"
            ));
        }
        runs(&body(&checks));
    }
}

#[test]
fn float_to_integer_finite_boundaries() {
    runs(&body(
        r#"
        if (9223372036854774784.0f64 as i64)!=9223372036854774784i64 {return false;}
        if (9223372036854775808.0f64 as i64)!=9223372036854775807i64 {return false;}
        if (-9223372036854775808.0f64 as i64)!=-9223372036854775808i64 {return false;}
        if (-9223372036854774784.0f64 as i64)!=-9223372036854774784i64 {return false;}
        if (-9223372036854777856.0f64 as i64)!=-9223372036854775808i64 {return false;}
        if (18446744073709549568.0f64 as u64)!=18446744073709549568u64 {return false;}
        if (18446744073709551616.0f64 as u64)!=18446744073709551615u64 {return false;}
        if (9223372036854775808.0f64 as u64)!=9223372036854775808u64 {return false;}
        if (9223372036854777856.0f64 as u64)!=9223372036854777856u64 {return false;}
        if (-128.9f64 as i8)!=-128i8 || (-127.9f64 as i8)!=-127i8 {return false;}
        if (255.9f64 as u8)!=255u8 || (256.0f64 as u8)!=255u8 {return false;}
        if (-128.9f32 as i8)!=-128i8 || (-127.9f32 as i8)!=-127i8 {return false;}
        if (255.9f32 as u8)!=255u8 || (256.0f32 as u8)!=255u8 {return false;}
    "#,
    ));
}

#[test]
fn integer_to_float_rounding_and_width_conversions() {
    for int in [
        "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize",
    ] {
        runs(&body(&format!(
            "let x=42{int}; if (x as f32)!=42.0f32 {{return false;}} if (x as f64)!=42.0f64 {{return false;}}"
        )));
    }
    runs(&body(
        r#"
        let inferred=1.25; let typed:f32=2.5;
        if inferred+0.75 != 2.0f64 || typed+0.5 != 3.0f32 {return false;}
        if 1_000.5f64 != 1.0005e3f64 {return false;}
        let tiny=1.401298464324817e-45f32;
        if (tiny as f64) != 1.401298464324817e-45f64 {return false;}
        let negative=-0.0f32;
        if 1.0f64/(negative as f64) >= 0.0f64 {return false;}
        let nan32=0.0f32/0.0f32; let wide=nan32 as f64;
        if wide==wide {return false;}
        let high=18446744073709551615u64;
        if (high as f64) != 18446744073709551616.0f64 {return false;}
        if (high as f32) != 18446744073709551616.0f32 {return false;}
        let tie=9007199254740993u64;
        if (tie as f64) != 9007199254740992.0f64 {return false;}
        let up=9007199254740995u64;
        if (up as f64) != 9007199254740996.0f64 {return false;}
        if (16777217u64 as f32) != 16777216.0f32 {return false;}
        if (-9223372036854775808i64 as f64) != -9223372036854775808.0f64 {return false;}
        let x=1.000000059604644775390625f64;
        if (x as f32) != 1.0f32 {return false;}
        let y=16777217.0f64;
        if ((y as f32) as f64) != 16777216.0f64 {return false;}
        let exact=1.5f32; if (exact as f64) != 1.5f64 {return false;}
        let z=-0.0f64; if 1.0f32/(z as f32) >= 0.0f32 {return false;}
        let huge=1.0e300f64; let inf=1.0f32/0.0f32;
        if (huge as f32) != inf {return false;}
        let small=1.0e-300f64; if (small as f32) != 0.0f32 {return false;}
        let nan=0.0f64/0.0f64; let narrow=nan as f32;
        if narrow==narrow {return false;}
    "#,
    ));
}

#[test]
fn aggregates_generic_copy_and_projected_updates() {
    runs(
        r#"
        struct Wrap<T>{value:T}
        impl<T:Copy> Clone for Wrap<T>{fn clone(&self)->Self{*self}}
        impl<T:Copy> Copy for Wrap<T>{}
        fn twice<T:Copy>(x:T)->(T,T){(x,x)}
        fn run()->bool{
            let p=twice::<f32>(1.5f32); if p.0+p.1 != 3.0f32{return false;}
            let q=twice::<Wrap<f64>>(Wrap{value:2.5f64});
            if q.0.value+q.1.value != 5.0f64{return false;}
            let mut a=[1.5f64,2.5f64]; let b=a; a[0]+=b[1];
            if a[0]!=4.0f64 || b[0]!=1.5f64{return false;}
            let mut tuple=(1.0f32,2.0f64); tuple.0*=3.0f32; tuple.1%=1.5f64;
            if tuple.0!=3.0f32 || tuple.1!=0.5f64{return false;}
            let mut w=Wrap{value:8.0f64}; w.value/=2.0f64;
            let r=&mut w.value; *r-=1.0f64; if w.value!=3.0f64{return false;}
            true
        }
        fn main(){if run(){println!("float oracle ok");}}
    "#,
    );
}

// Expected bits are stock rustc results on Linux x86-64 (glibc libm).
#[test]
fn f64_math_methods_and_constants_match_stock_bits() {
    for checks in [
        "if (2.0f64.powf(0.5)).to_bits()!=0x3ff6a09e667f3bcdu64{return false;}if (2.0f64.powf(10.0)).to_bits()!=0x4090000000000000u64{return false;}if ((-8.0f64).powf(3.0)).to_bits()!=0xc080000000000000u64{return false;}if ((-8.0f64).powf(0.5)).to_bits()!=0xfff8000000000000u64{return false;}if (0.0f64.powf(-1.0)).to_bits()!=0x7ff0000000000000u64{return false;}if (1.5f64.powf(-700.25)).to_bits()!=0x2654d2543a1eb14du64{return false;}if (1e-310f64.powf(0.75)).to_bits()!=0x0fa922f895935598u64{return false;}if (10.0f64.powf(308.5)).to_bits()!=0x7ff0000000000000u64{return false;}if (0.999f64.powf(1e9)).to_bits()!=0x0000000000000000u64{return false;}if (3.0f64.powf(-0.0)).to_bits()!=0x3ff0000000000000u64{return false;}if (1.0f64.exp()).to_bits()!=0x4005bf0a8b145769u64{return false;}if ((-745.2f64).exp()).to_bits()!=0x0000000000000000u64{return false;}if (709.9f64.exp()).to_bits()!=0x7ff0000000000000u64{return false;}if (0.1f64.exp()).to_bits()!=0x3ff1aec7b35a00d4u64{return false;}if ((-0.0f64).exp()).to_bits()!=0x3ff0000000000000u64{return false;}if (3.5f64.exp2()).to_bits()!=0x4026a09e667f3bcdu64{return false;}if ((-1074.5f64).exp2()).to_bits()!=0x0000000000000001u64{return false;}if (1023.99f64.exp2()).to_bits()!=0x7fefc769e9b9c396u64{return false;}if (10.0f64.ln()).to_bits()!=0x40026bb1bbb55516u64{return false;}if (0.5f64.ln()).to_bits()!=0xbfe62e42fefa39efu64{return false;}if (1.0000001f64.ln()).to_bits()!=0x3e7ad7f2847b6492u64{return false;}if (0.0f64.ln()).to_bits()!=0xfff0000000000000u64{return false;}if ((-1.0f64).ln()).to_bits()!=0xfff8000000000000u64{return false;}if (5e-324f64.ln()).to_bits()!=0xc0874385446d71c3u64{return false;}",
        "if (8.0f64.log2()).to_bits()!=0x4008000000000000u64{return false;}if (0.3f64.log2()).to_bits()!=0xbffbca9c6f53897bu64{return false;}if (1.01f64.log2()).to_bits()!=0x3f8d664ecee35b7fu64{return false;}if (1000.0f64.log10()).to_bits()!=0x4008000000000000u64{return false;}if (0.007f64.log10()).to_bits()!=0xc0013d3d3d21ccf0u64{return false;}if ((-3.0f64).log10()).to_bits()!=0x7ff8000000000000u64{return false;}if (81.0f64.log(3.0)).to_bits()!=0x4010000000000000u64{return false;}if (3.0f64.hypot(4.0)).to_bits()!=0x4014000000000000u64{return false;}if (1e200f64.hypot(1e200)).to_bits()!=0x697d8f9811335b57u64{return false;}if (1e-200f64.hypot(3e-200)).to_bits()!=0x16835d5244b69495u64{return false;}if (1.5f64.hypot(-2.25)).to_bits()!=0x4005a22073490377u64{return false;}if (0.1f64.mul_add(10.0,-1.0)).to_bits()!=0x3c90000000000000u64{return false;}if (1e308f64.mul_add(10.0,-1e308)).to_bits()!=0x7ff0000000000000u64{return false;}if (3.0f64.mul_add(-0.0,0.0)).to_bits()!=0x0000000000000000u64{return false;}if (5e-324f64.mul_add(0.5,5e-324)).to_bits()!=0x0000000000000002u64{return false;}if (2.5f64.floor()).to_bits()!=0x4000000000000000u64{return false;}if ((-2.5f64).floor()).to_bits()!=0xc008000000000000u64{return false;}if (2.5f64.ceil()).to_bits()!=0x4008000000000000u64{return false;}if ((-2.5f64).ceil()).to_bits()!=0xc000000000000000u64{return false;}if (2.5f64.round()).to_bits()!=0x4008000000000000u64{return false;}if ((-2.5f64).round()).to_bits()!=0xc008000000000000u64{return false;}if (0.49999999999999994f64.round()).to_bits()!=0x0000000000000000u64{return false;}if (2.5f64.round_ties_even()).to_bits()!=0x4000000000000000u64{return false;}if ((-0.3f64).trunc()).to_bits()!=0x8000000000000000u64{return false;}",
        "if ((-0.3f64).ceil()).to_bits()!=0x8000000000000000u64{return false;}if (1e300f64.floor()).to_bits()!=0x7e37e43c8800759cu64{return false;}if ((-7.75f64).fract()).to_bits()!=0xbfe8000000000000u64{return false;}if ((-0.0f64).signum()).to_bits()!=0xbff0000000000000u64{return false;}if (3.0f64.signum()).to_bits()!=0x3ff0000000000000u64{return false;}if (2.0f64.copysign(-0.0)).to_bits()!=0xc000000000000000u64{return false;}if (7.5f64.clamp(0.0,3.0)).to_bits()!=0x4008000000000000u64{return false;}if ((-7.5f64).clamp(-1.0,3.0)).to_bits()!=0xbff0000000000000u64{return false;}if (4.0f64.recip()).to_bits()!=0x3fd0000000000000u64{return false;}if (1.1f64.powi(7)).to_bits()!=0x3fff2df1fb5a7ed7u64{return false;}if (2.0f64.powi(-3)).to_bits()!=0x3fc0000000000000u64{return false;}if (0.0f64.powi(0)).to_bits()!=0x3ff0000000000000u64{return false;}if ((-1.5f64).powi(31)).to_bits()!=0xc1118e2a5afb5158u64{return false;}if (180.0f64.to_radians()).to_bits()!=0x400921fb54442d18u64{return false;}if (1.0f64.to_degrees()).to_bits()!=0x404ca5dc1a63c1f8u64{return false;}if ((-7.5f64).div_euclid(2.0)).to_bits()!=0xc010000000000000u64{return false;}if ((-7.5f64).rem_euclid(2.0)).to_bits()!=0x3fe0000000000000u64{return false;}if (7.5f64.rem_euclid(-2.0)).to_bits()!=0x3ff8000000000000u64{return false;}if (f64::MAX).to_bits()!=0x7fefffffffffffffu64{return false;}if (f64::MIN).to_bits()!=0xffefffffffffffffu64{return false;}if (f64::EPSILON).to_bits()!=0x3cb0000000000000u64{return false;}if (f64::MIN_POSITIVE).to_bits()!=0x0010000000000000u64{return false;}if (f64::INFINITY).to_bits()!=0x7ff0000000000000u64{return false;}if (f64::NEG_INFINITY).to_bits()!=0xfff0000000000000u64{return false;}",
        "if (f64::NAN).to_bits()!=0x7ff8000000000000u64{return false;}if (std::f64::consts::PI).to_bits()!=0x400921fb54442d18u64{return false;}if (std::f64::consts::E).to_bits()!=0x4005bf0a8b145769u64{return false;}if (core::f64::consts::LN_2).to_bits()!=0x3fe62e42fefa39efu64{return false;}if (std::f64::consts::SQRT_2).to_bits()!=0x3ff6a09e667f3bcdu64{return false;}if (std::f64::consts::TAU).to_bits()!=0x401921fb54442d18u64{return false;}if (std::f64::MAX).to_bits()!=0x7fefffffffffffffu64{return false;}if ((f64::NAN).max(1.0)).to_bits()!=0x3ff0000000000000u64{return false;}if (f64::NAN.powf(0.0)).to_bits()!=0x3ff0000000000000u64{return false;}if (f64::sqrt(2.0)).to_bits()!=0x3ff6a09e667f3bcdu64{return false;}if (f64::powf(2.0,0.5)).to_bits()!=0x3ff6a09e667f3bcdu64{return false;}if (f64::max(1.0,2.0)).to_bits()!=0x4000000000000000u64{return false;}if (f64::abs(-3.5)).to_bits()!=0x400c000000000000u64{return false;}if (f64::floor(2.5)).to_bits()!=0x4000000000000000u64{return false;}if (f64::hypot(3.0,4.0)).to_bits()!=0x4014000000000000u64{return false;}",
    ] {
        runs(&body(checks));
    }
}

#[test]
fn f32_math_methods_and_constants_match_stock_bits() {
    for checks in [
        "if (2.0f32.powf(0.5)).to_bits()!=0x3fb504f3u32{return false;}if ((-8.0f32).powf(3.0)).to_bits()!=0xc4000000u32{return false;}if ((-8.0f32).powf(0.5)).to_bits()!=0xffc00000u32{return false;}if (1.5f32.powf(-200.25)).to_bits()!=0x04e8871cu32{return false;}if (10.0f32.powf(38.4)).to_bits()!=0x7f3cf958u32{return false;}if (1e-40f32.powf(0.75)).to_bits()!=0x0da24235u32{return false;}if (0.0f32.powf(-3.0)).to_bits()!=0x7f800000u32{return false;}if (1.0f32.exp()).to_bits()!=0x402df854u32{return false;}if ((-103.5f32).exp()).to_bits()!=0x00000001u32{return false;}if (88.7f32.exp()).to_bits()!=0x7f7a37fcu32{return false;}if (3.5f32.exp2()).to_bits()!=0x413504f3u32{return false;}if ((-149.5f32).exp2()).to_bits()!=0x00000001u32{return false;}if (10.0f32.ln()).to_bits()!=0x40135d8eu32{return false;}if (1.0001f32.ln()).to_bits()!=0x38d1bd51u32{return false;}if (0.0f32.ln()).to_bits()!=0xff800000u32{return false;}if ((-1.0f32).ln()).to_bits()!=0xffc00000u32{return false;}if (8.0f32.log2()).to_bits()!=0x40400000u32{return false;}if (0.3f32.log2()).to_bits()!=0xbfde54e3u32{return false;}if (1000.0f32.log10()).to_bits()!=0x40400000u32{return false;}if (81.0f32.log(3.0)).to_bits()!=0x40800000u32{return false;}if (0.5f32.sin()).to_bits()!=0x3ef57744u32{return false;}if (0.5f32.cos()).to_bits()!=0x3f60a940u32{return false;}if (3.0f32.sin()).to_bits()!=0x3e1081c3u32{return false;}if (100.0f32.cos()).to_bits()!=0x3f5cc0eeu32{return false;}",
        "if (1e10f32.sin()).to_bits()!=0xbef99a64u32{return false;}if ((-1e30f32).cos()).to_bits()!=0xbf1c9222u32{return false;}if (1e-5f32.sin()).to_bits()!=0x3727c5acu32{return false;}if ((1.0f32/0.0).sin()).to_bits()!=0xffc00000u32{return false;}if (2.0f32.sin_cos().0).to_bits()!=0x3f68c7b7u32{return false;}if (2.0f32.sin_cos().1).to_bits()!=0xbed51133u32{return false;}if (3.0f32.hypot(4.0)).to_bits()!=0x40a00000u32{return false;}if (1e30f32.hypot(1e30)).to_bits()!=0x718ecc90u32{return false;}if (0.1f32.mul_add(10.0,-1.0)).to_bits()!=0x32800000u32{return false;}if (1e38f32.mul_add(10.0,-1e38)).to_bits()!=0x7f800000u32{return false;}if (2.5f32.floor()).to_bits()!=0x40000000u32{return false;}if ((-2.5f32).ceil()).to_bits()!=0xc0000000u32{return false;}if (2.5f32.round()).to_bits()!=0x40400000u32{return false;}if (0.49999997f32.round()).to_bits()!=0x00000000u32{return false;}if (3.5f32.round_ties_even()).to_bits()!=0x40800000u32{return false;}if ((-0.3f32).trunc()).to_bits()!=0x80000000u32{return false;}if ((-7.75f32).fract()).to_bits()!=0xbf400000u32{return false;}if ((-0.0f32).signum()).to_bits()!=0xbf800000u32{return false;}if (2.0f32.copysign(-1.0)).to_bits()!=0xc0000000u32{return false;}if (7.5f32.clamp(0.0,3.0)).to_bits()!=0x40400000u32{return false;}if (1.1f32.powi(7)).to_bits()!=0x3ff96f92u32{return false;}if (2.0f32.powi(-3)).to_bits()!=0x3e000000u32{return false;}if (180.0f32.to_radians()).to_bits()!=0x40490fdbu32{return false;}if (1.0f32.to_degrees()).to_bits()!=0x42652ee1u32{return false;}",
        "if ((-7.5f32).rem_euclid(2.0)).to_bits()!=0x3f000000u32{return false;}if ((-7.5f32).div_euclid(2.0)).to_bits()!=0xc0800000u32{return false;}if (f32::MAX).to_bits()!=0x7f7fffffu32{return false;}if (f32::MIN).to_bits()!=0xff7fffffu32{return false;}if (f32::EPSILON).to_bits()!=0x34000000u32{return false;}if (f32::MIN_POSITIVE).to_bits()!=0x00800000u32{return false;}if (f32::INFINITY).to_bits()!=0x7f800000u32{return false;}if (f32::NEG_INFINITY).to_bits()!=0xff800000u32{return false;}if (f32::NAN).to_bits()!=0x7fc00000u32{return false;}if (std::f32::consts::PI).to_bits()!=0x40490fdbu32{return false;}if (std::f32::consts::E).to_bits()!=0x402df854u32{return false;}if (core::f32::consts::FRAC_1_SQRT_2).to_bits()!=0x3f3504f3u32{return false;}if (std::f32::EPSILON).to_bits()!=0x34000000u32{return false;}if (f32::sqrt(2.0)).to_bits()!=0x3fb504f3u32{return false;}if (f32::powf(2.0,0.5)).to_bits()!=0x3fb504f3u32{return false;}if (f32::sin(1.0)).to_bits()!=0x3f576aa4u32{return false;}",
    ] {
        runs(&body(checks));
    }
}

#[test]
fn float_math_in_generic_closure_and_function_value_positions() {
    runs(
        "fn area(r:f64)->f64{std::f64::consts::PI*r.powi(2)} \
         fn norm(v:&[f64])->f64{v.iter().map(|x|x.powf(2.0)).sum::<f64>().sqrt()} \
         fn run()->bool{ \
         let v=vec![3.0f64,-4.0,12.0]; \
         if norm(&v)!=13.0{return false;} \
         if v.iter().cloned().fold(f64::NEG_INFINITY,f64::max)!=12.0{return false;} \
         if v.iter().map(|x|x.abs().sqrt().floor()).sum::<f64>()!=6.0{return false;} \
         if area(2.0).to_bits()!=0x402921fb54442d18u64{return false;} \
         let x:f64=2.0; let y=x.powf(0.5); if y.to_bits()!=0x3ff6a09e667f3bcdu64{return false;} \
         let z:f32=9.0; if z.ln().exp().round()!=9.0{return false;} \
         if 7.clamp(0,3)!=3||5i32.max(9)!=9||(-7i32).rem_euclid(4)!=1{return false;} \
         if !(f64::NAN.is_nan()&&f32::NAN.is_nan()&&f64::INFINITY.is_infinite()){return false;} \
         if f64::MANTISSA_DIGITS!=53||f32::MAX_EXP!=128||f64::MIN_10_EXP!=-307{return false;} \
         true} fn main(){if run(){println!(\"float oracle ok\");}}",
    );
}

#[test]
fn rejects_invalid_float_math_uses() {
    for (label, expression) in [
        ("powf arity", "2.0f64.powf()"),
        ("powf mixed width", "2.0f64.powf(1.0f32)"),
        ("powi float exponent", "2.0f64.powi(2.0)"),
        ("floor argument", "2.5f32.floor(1.0)"),
        ("float method on integer", "2i32.powf(2.0)"),
        ("float method on bool", "true.sqrt()"),
        ("unknown float method", "2.0f64.cube_root_of_doom()"),
        ("f64 sin stays unsupported", "1.0f64.sin()"),
        ("f64 atan2 stays unsupported", "1.0f64.atan2(2.0)"),
        ("unknown float constant", "f64::NOT_A_CONSTANT"),
        ("unknown consts item", "std::f64::consts::NOPE"),
        (
            "constant width mismatch",
            "{let x:f32=std::f64::consts::PI;x}",
        ),
        ("clamp mixed width", "1.0f64.clamp(0.0f32,1.0f32)"),
    ] {
        rejects(label, &format!("fn main(){{let x={expression};}}"));
    }
}

#[test]
fn rejects_invalid_float_operators_and_mixed_types() {
    for (label, expression) in [
        ("bit and", "1.0f32 & 2.0f32"),
        ("bit or", "1.0f64 | 2.0f64"),
        ("bit xor", "1.0f32 ^ 2.0f32"),
        ("shift left", "1.0f64 << 1u32"),
        ("shift right", "1.0f32 >> 1u32"),
        ("not", "!1.0f64"),
        ("mixed widths", "1.0f32 + 2.0f64"),
        ("mixed integer", "1.0f64 + 2i32"),
        ("mixed comparison", "1.0f32 < 2.0f64"),
        ("mixed equality", "1.0f64 == 1i32"),
        ("bool cast", "1.0f32 as bool"),
        ("char cast", "1.0f64 as char"),
    ] {
        rejects(label, &format!("fn main(){{let x={expression};}}"));
    }
    for source in [
        "fn main(){let mut x=1.0f32; x+=2.0f64;}",
        "fn main(){let mut x=1.0f64; x&=2.0f64;}",
        "fn main(){let x:f32=1.0f64;}",
        "fn main(){let x=1.0f16;}",
        "fn main(){let x=1.0f128;}",
        "fn main(){let x=1.0e999f32;}",
        "fn main(){let x=1.0e999f64;}",
        "fn main(){let x=1.0e+f64;}",
    ] {
        rejects("float literal/type rejection", source);
    }
}
