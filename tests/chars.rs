//! Bounded char scalar acceptance, frozen independently before native execution.
//! Stock: /usr/bin/rustc 1.98.1, edition 2024, 10 executable positives / 26 rejections.
//! Receipt: /home/jeremy/.jcode/scratch/chars-stock-20261018/results.json
//! Receipt SHA256: 7bc819a16b51543c63923f0f0f6f54de9b9820fd7519789e2e7c09f4c5b12e53
//! No char formatting, casts, literal patterns or range patterns are claimed.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::{
    Key,
    compiler::{self, frontend},
};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
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
            let path = base.join(format!("jr-chars-{}-{id}", std::process::id()));
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    Live,
    Syntax,
    Type,
}
struct Case {
    name: &'static str,
    source: &'static str,
    bytes: usize,
    sha256: &'static str,
    boundary: Boundary,
    stock_compile_code: i32,
    stock_compile_stderr: &'static str,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: r###"ascii_inferred_typed"###,
            source: r###"fn run()->bool{let a='a';let b:char='z';a!=b&&a<b&&b>a&&a<=a&&b>=b&&a=='a'}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 124,
            sha256: r###"5ff662cea43958eedb49ee0c07570ae98f129dcf659babaf86b9c25229f52a3b"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"unicode_bmp_astral"###,
            source: r###"fn run()->bool{let a='é';let b='中';let c='🦀';a<b&&b<c&&c=='\u{1f980}'}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 125,
            sha256: r###"c3882ef1aa0bf0b5f53c71dd5f072e86ec93154fa3ae63771c7f3fa739f6cc5b"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"scalar_boundaries"###,
            source: r###"fn run()->bool{'\0'<'\u{d7ff}'&&'\u{d7ff}'<'\u{e000}'&&'\u{e000}'<'\u{10ffff}'}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 128,
            sha256: r###"1782e3c9ac5a4c950687d27b4804ad1b5b0b0fe9dfa92d84f9b6c43e6888aebc"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"escapes"###,
            source: r###"fn run()->bool{'\n'=='\u{a}'&&'\r'=='\u{d}'&&'\t'=='\u{9}'&&'\0'=='\u{0}'&&'\\'=='\u{5c}'&&'\''=='\u{27}'&&'\"'=='\u{22}'&&'\x41'=='A'}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 184,
            sha256: r###"57ce005d4748e8318a5e6676ba3e1a7026b19d7f9e5e0fecd58b43488947d6ea"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"unicode_escape_forms"###,
            source: r###"fn run()->bool{'\u{000041}'=='A'&&'\u{1_F980}'=='🦀'&&'\x7f'=='\u{7f}'}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 122,
            sha256: r###"9e07e2c598d8c05d6b6ccd169951313cc73a270e74b9dd2b9ac2899e68c4a050"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"params_results"###,
            source: r###"fn identity(x:char)->char{x}fn choose(a:char,b:char)->char{if a<b{b}else{a}}fn run()->bool{identity('🦀')=='🦀'&&choose('a','z')=='z'}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 187,
            sha256: r###"95559ce12ef2fb3e7bc92d15feac11ee9f259ae1b53516840b55e7af4119f67d"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"aggregate_fields"###,
            source: r###"struct Pair{a:char,b:char}fn run()->bool{let p=Pair{a:'🦀',b:'a'};let t=(p.a,p.b);let a=[t.0,t.1];a[0usize]=='🦀'&&a[1usize]=='a'}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 183,
            sha256: r###"1cce11bc59418d5ea690036fa98411dfb6590e799979096cf3de1ee18d740725"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"copy_clone_sized"###,
            source: r###"fn twice<T:Copy>(x:T)->(T,T){(x,x)}fn sized<T:Sized>(x:T)->T{x}fn run()->bool{let c='🦀';let p=twice(c);let d=<char as Clone>::clone(&c);sized(d)==c&&p.0==c&&p.1==c}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 216,
            sha256: r###"430fe1a5175b96eaa6050aa9c05f525edcedaadb2e57ed5769d71664c61416d1"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"mutation_and_reference"###,
            source: r###"fn run()->bool{let mut c='a';c='🦀';let r=&c;*r=='🦀'}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 107,
            sha256: r###"141a4f826c30fc8086c652d062b991649eabe7f19627bb0d0b91570c5faf0ff4"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"unused_char_function"###,
            source: r###"fn unused(x:char)->char{x}fn run()->bool{true}fn main(){if run(){println!("char oracle ok");}}
"###,
            bytes: 95,
            sha256: r###"8a2c6936f7e27e04b2c418b1be8f1c72c182214a10b058b5fb827a7c04d6ac25"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"invalid_empty"###,
            source: r###"fn main(){let c='';}
"###,
            bytes: 21,
            sha256: r###"8618004eaeff5d68ad342dd956f4aac01cc35196c2a05e0f6e6d189c4d19f626"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: empty character literal
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_empty.rs:1:18
  |
1 | fn main(){let c='';}
  |                  ^ empty character literal

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"invalid_multiple"###,
            source: r###"fn main(){let c='ab';}
"###,
            bytes: 23,
            sha256: r###"edf86b95623c9b5eb06c55b0371f9d205f314607566358bc03ba77ac7a757f0f"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: character literal may only contain one codepoint
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_multiple.rs:1:17
  |
1 | fn main(){let c='ab';}
  |                 ^^^^
  |
help: if you meant to write a string literal, use double quotes
  |
1 - fn main(){let c='ab';}
1 + fn main(){let c="ab";}
  |

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"invalid_multiple_unicode"###,
            source: r###"fn main(){let c='é🦀';}
"###,
            bytes: 27,
            sha256: r###"947601d8823f578fde926723d9aded15c8beff0ac1c5430a98b5f488607b23f1"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0762]: unterminated character literal
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_multiple_unicode.rs:1:20
  |
1 | fn main(){let c='é🦀';}
  |                     ^^^
  |
help: if you meant to write a string literal, use double quotes
  |
1 - fn main(){let c='é🦀';}
1 + fn main(){let c="é🦀";}
  |

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0762`.
"###,
        },
        Case {
            name: r###"invalid_surrogate_low"###,
            source: r###"fn main(){let c='\u{d800}';}
"###,
            bytes: 29,
            sha256: r###"487ddb3311c601a2104c5007bc904a9bb1ddc60f037aae658a42f8a7b523fd5d"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: invalid unicode character escape
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_surrogate_low.rs:1:18
  |
1 | fn main(){let c='\u{d800}';}
  |                  ^^^^^^^^ invalid escape
  |
  = help: unicode escape must not be a surrogate

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"invalid_surrogate_high"###,
            source: r###"fn main(){let c='\u{dfff}';}
"###,
            bytes: 29,
            sha256: r###"2db3c641b030af8cd781ebb4bb27604c32f577a89ac6b189233e4517f554c259"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: invalid unicode character escape
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_surrogate_high.rs:1:18
  |
1 | fn main(){let c='\u{dfff}';}
  |                  ^^^^^^^^ invalid escape
  |
  = help: unicode escape must not be a surrogate

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"invalid_out_of_range"###,
            source: r###"fn main(){let c='\u{110000}';}
"###,
            bytes: 31,
            sha256: r###"e82b85727514d52a2f11f7d344da4154ef394fccc3b41e59ac14ec0d4dd283a0"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: invalid unicode character escape
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_out_of_range.rs:1:18
  |
1 | fn main(){let c='\u{110000}';}
  |                  ^^^^^^^^^^ invalid escape
  |
  = help: unicode escape must be at most 10FFFF

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"invalid_escape_unknown"###,
            source: r###"fn main(){let c='\q';}
"###,
            bytes: 23,
            sha256: r###"4c4085937c24fa4b8c54ee6ddf1b9f8c11f7d4492460821fd266f782a8829618"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: unknown character escape: `q`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_escape_unknown.rs:1:19
  |
1 | fn main(){let c='\q';}
  |                   ^ unknown character escape
  |
  = help: for more information, visit <https://doc.rust-lang.org/reference/tokens.html#literals>
help: if you meant to write a literal backslash (perhaps escaping in a regular expression), consider a raw string literal
  |
1 - fn main(){let c='\q';}
1 + fn main(){let c=r"\q";}
  |

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"invalid_escape_nonascii_hex"###,
            source: r###"fn main(){let c='\x80';}
"###,
            bytes: 25,
            sha256: r###"119eb1a918df05c2d967279f92fac104bd09bb78425fafc62a765c90acd95b48"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: out of range hex escape
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_escape_nonascii_hex.rs:1:18
  |
1 | fn main(){let c='\x80';}
  |                  ^^^^ must be a character in the range [\x00-\x7f]
  |
  = help: if you want to write a byte literal, use `b'\x80'`
  = help: if you want to write a Unicode character, use `'\u{80}'`

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"invalid_escape_empty"###,
            source: r###"fn main(){let c='\u{}';}
"###,
            bytes: 25,
            sha256: r###"f6e1e019823dfc6ede3981e8c1998a7fd5069fafac12e28fa1e3d803ccdf743b"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: empty unicode escape
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_escape_empty.rs:1:18
  |
1 | fn main(){let c='\u{}';}
  |                  ^^^^ this escape must have at least 1 hex digit

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"invalid_escape_too_long"###,
            source: r###"fn main(){let c='\u{0000000}';}
"###,
            bytes: 32,
            sha256: r###"0fbd3715b4899863608733bd5ad1d21f07ef821d3b0b7e82f267eb92a612f988"###,
            boundary: Boundary::Syntax,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error: overlong unicode escape
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/invalid_escape_too_long.rs:1:18
  |
1 | fn main(){let c='\u{0000000}';}
  |                  ^^^^^^^^^^^ must have at most 6 hex digits

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"reject_eq_int"###,
            source: r###"fn bad(){let c='a'==97;}fn main(){}
"###,
            bytes: 36,
            sha256: r###"ea6aa38b4af774af2ede7bb15c38b81727975ce0fc240ed74783cbc2ae359151"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_eq_int.rs:1:21
  |
1 | fn bad(){let c='a'==97;}fn main(){}
  |                ---  ^^ expected `char`, found `u8`
  |                |
  |                expected because this is `char`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
        },
        Case {
            name: r###"reject_ne_int"###,
            source: r###"fn bad(){let c='a'!=97u32;}fn main(){}
"###,
            bytes: 39,
            sha256: r###"f75ea2dfc430a8ad7e0042fbf7044a2b1d9b66c0daded6f876f4ff2d576c869c"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_ne_int.rs:1:21
  |
1 | fn bad(){let c='a'!=97u32;}fn main(){}
  |                ---  ^^^^^ expected `char`, found `u32`
  |                |
  |                expected because this is `char`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
        },
        Case {
            name: r###"reject_lt_int"###,
            source: r###"fn bad(){let c='a'<97;}fn main(){}
"###,
            bytes: 35,
            sha256: r###"3dbf3ccf74efc5c78f9fb2f0cea68e3060a9a99b98c2e1bef2995f77d11405ea"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_lt_int.rs:1:20
  |
1 | fn bad(){let c='a'<97;}fn main(){}
  |                --- ^^ expected `char`, found `u8`
  |                |
  |                expected because this is `char`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
        },
        Case {
            name: r###"reject_int_gt"###,
            source: r###"fn bad(){let c=97>'a';}fn main(){}
"###,
            bytes: 35,
            sha256: r###"6b241342f3cebee594317588a837ce84e8e3f7ac3e6ee3bda6645bedbaa30056"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_int_gt.rs:1:19
  |
1 | fn bad(){let c=97>'a';}fn main(){}
  |                   ^^^ expected integer, found `char`

error[E0277]: can't compare `{integer}` with `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_int_gt.rs:1:18
  |
1 | fn bad(){let c=97>'a';}fn main(){}
  |                  ^ no implementation for `{integer} < char` and `{integer} > char`
  |
  = help: the trait `PartialOrd<char>` is not implemented for `{integer}`
  = help: the following other types implement trait `PartialOrd<Rhs>`:
            f128
            f16
            f32
            f64
            i128
            i16
            i32
            i64
          and 8 others

error: aborting due to 2 previous errors

Some errors have detailed explanations: E0277, E0308.
For more information about an error, try `rustc --explain E0277`.
"###,
        },
        Case {
            name: r###"reject_add_char"###,
            source: r###"fn bad(){let c='a'+'b';}fn main(){}
"###,
            bytes: 36,
            sha256: r###"41d41981efba8db51eb1b3b28c2b3b817fc58d027fe7b3867a3b08aa7a3df893"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0369]: cannot add `char` to `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_add_char.rs:1:19
  |
1 | fn bad(){let c='a'+'b';}fn main(){}
  |                ---^--- char
  |                |
  |                char

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0369`.
"###,
        },
        Case {
            name: r###"reject_sub_char"###,
            source: r###"fn bad(){let c='a'-'b';}fn main(){}
"###,
            bytes: 36,
            sha256: r###"972a7ff18557d99d0fa4ffcb8d4ba3a66adcd8833238748754dd5d65c5e9fc25"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0369]: cannot subtract `char` from `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_sub_char.rs:1:19
  |
1 | fn bad(){let c='a'-'b';}fn main(){}
  |                ---^--- char
  |                |
  |                char

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0369`.
"###,
        },
        Case {
            name: r###"reject_mul_char"###,
            source: r###"fn bad(){let c='a'*'b';}fn main(){}
"###,
            bytes: 36,
            sha256: r###"863e77bd8536b0db108f4de71f890c234aed37eb4adcfb3a2c36ee88c7087749"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0369]: cannot multiply `char` by `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_mul_char.rs:1:19
  |
1 | fn bad(){let c='a'*'b';}fn main(){}
  |                ---^--- char
  |                |
  |                char

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0369`.
"###,
        },
        Case {
            name: r###"reject_div_char"###,
            source: r###"fn bad(){let c='a'/'b';}fn main(){}
"###,
            bytes: 36,
            sha256: r###"85fa24c0b8315e7572d9a7dfc928fbb54507d70b919dc43892b79e88769e97a0"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0369]: cannot divide `char` by `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_div_char.rs:1:19
  |
1 | fn bad(){let c='a'/'b';}fn main(){}
  |                ---^--- char
  |                |
  |                char

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0369`.
"###,
        },
        Case {
            name: r###"reject_rem_char"###,
            source: r###"fn bad(){let c='a'%'b';}fn main(){}
"###,
            bytes: 36,
            sha256: r###"1f021c7e29d5dfef7d2cd9a18b3a140465676720aa7afd8f4d0d83845e94542d"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0369]: cannot calculate the remainder of `char` divided by `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_rem_char.rs:1:19
  |
1 | fn bad(){let c='a'%'b';}fn main(){}
  |                ---^--- char
  |                |
  |                char

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0369`.
"###,
        },
        Case {
            name: r###"reject_add_int"###,
            source: r###"fn bad(){let c='a'+1;}fn main(){}
"###,
            bytes: 34,
            sha256: r###"d013118385b69ad3468fb70611f775f6d4e693803e17e61387f42cb01d620ff6"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0369]: cannot add `{integer}` to `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_add_int.rs:1:19
  |
1 | fn bad(){let c='a'+1;}fn main(){}
  |                ---^- {integer}
  |                |
  |                char

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0369`.
"###,
        },
        Case {
            name: r###"reject_bitand"###,
            source: r###"fn bad(){let c='a'&'b';}fn main(){}
"###,
            bytes: 36,
            sha256: r###"38c1655b2b4af846295dae07395228269cbda1fa39de1846bad59e7574817b3a"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0369]: no implementation for `char & char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_bitand.rs:1:19
  |
1 | fn bad(){let c='a'&'b';}fn main(){}
  |                ---^--- char
  |                |
  |                char

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0369`.
"###,
        },
        Case {
            name: r###"reject_negate"###,
            source: r###"fn bad(){let c=-'a';}fn main(){}
"###,
            bytes: 33,
            sha256: r###"7764ac39900d47d21ac04b29eace7ca6174372fe7396b6c43183d0501ea32f64"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0600]: cannot apply unary operator `-` to type `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_negate.rs:1:16
  |
1 | fn bad(){let c=-'a';}fn main(){}
  |                ^^^^ cannot apply unary operator `-`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0600`.
"###,
        },
        Case {
            name: r###"reject_not"###,
            source: r###"fn bad(){let c=!'a';}fn main(){}
"###,
            bytes: 33,
            sha256: r###"53460fde0a82c77a32212540c1fdb23926a87fc777fdac2be912628ef348f4a1"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0600]: cannot apply unary operator `!` to type `char`
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_not.rs:1:16
  |
1 | fn bad(){let c=!'a';}fn main(){}
  |                ^^^^ cannot apply unary operator `!`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0600`.
"###,
        },
        Case {
            name: r###"reject_result_int"###,
            source: r###"fn bad()->char{97}fn main(){}
"###,
            bytes: 30,
            sha256: r###"72cc5b0a845d7302b0a4a61c94364149f21da9e46230481d29209558ed483e3a"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_result_int.rs:1:16
  |
1 | fn bad()->char{97}fn main(){}
  |           ---- ^^ expected `char`, found `u8`
  |           |
  |           expected `char` because of return type

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
        },
        Case {
            name: r###"reject_parameter_int"###,
            source: r###"fn f(c:char){}fn main(){f(97);}
"###,
            bytes: 32,
            sha256: r###"abbccadbcc52d4fcb609e03529dd561c14afa5ac211f18fdc824c622738e20ec"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_parameter_int.rs:1:27
  |
1 | fn f(c:char){}fn main(){f(97);}
  |                         - ^^ expected `char`, found `u8`
  |                         |
  |                         arguments to this function are incorrect
  |
note: function defined here
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_parameter_int.rs:1:4
  |
1 | fn f(c:char){}fn main(){f(97);}
  |    ^ ------

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
        },
        Case {
            name: r###"reject_assignment_int"###,
            source: r###"fn main(){let mut c='a';c=97;}
"###,
            bytes: 31,
            sha256: r###"b6e988b0630040943d3658ae58669abdf2f6c40e9a1ade9501527b680e717b94"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/chars-stock-20261018/sources/reject_assignment_int.rs:1:27
  |
1 | fn main(){let mut c='a';c=97;}
  |                     ---   ^^ expected `char`, found `u8`
  |                     |
  |                     expected due to this value

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
        },
    ]
}

#[derive(Default)]
struct Failures(Vec<String>);
impl Failures {
    fn cell(&mut self, label: String, f: impl FnOnce()) {
        if let Err(error) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            let detail = error
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "non-string panic".into());
            self.0.push(format!("{label}: {detail}"));
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} failed cells:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}
fn stock_all() {
    static STOCK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STOCK.get_or_init(|| {
        let mut failures = Failures::default();
        for case in cases() {
            failures.cell(format!("stock {}", case.name), || {
                let scratch = Scratch::new();
                let input = scratch.source(case.source);
                let output = scratch.path("stock");
                assert_eq!(case.source.len(), case.bytes);
                assert!(case.source.ends_with('\n'));
                let hash = capture(Command::new("/usr/bin/sha256sum").arg(&input), &scratch, 10);
                success(&hash, "source SHA256");
                assert_eq!(
                    String::from_utf8(hash.stdout).unwrap(),
                    format!("{}  {}\n", case.sha256, input.display())
                );
                let result = capture(
                    Command::new("/usr/bin/rustc")
                        .env_remove("RUSTC_BOOTSTRAP")
                        .arg(&input)
                        .args(["--edition=2024", "-A", "warnings"])
                        .arg("-o")
                        .arg(&output),
                    &scratch,
                    30,
                );
                assert_eq!(
                    result.status.code(),
                    Some(case.stock_compile_code),
                    "{result:?}"
                );
                assert!(result.stdout.is_empty());
                if case.boundary == Boundary::Live {
                    assert!(result.stderr.is_empty());
                    runtime(&output, &scratch);
                } else {
                    assert!(!result.stderr.is_empty());
                    assert!(!case.stock_compile_stderr.is_empty());
                    assert!(!output.exists());
                }
            });
        }
        failures.finish();
    });
}
fn runtime(output: &Path, scratch: &Scratch) {
    let bytes = fs::read(output).unwrap();
    assert!(bytes.len() >= 64);
    assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
    let result = capture(&mut Command::new(output), scratch, 5);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert_eq!(result.stdout, b"char oracle ok\n");
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn rejection(case: &Case, text: &str, graph: bool) {
    assert!(!text.contains("panicked at"), "{}: {text}", case.name);
    assert!(!text.contains("Unsupported"), "{}: {text}", case.name);
    let expected = match case.name {
        "invalid_empty" | "invalid_multiple" => {
            "character literal must contain exactly one character"
        }
        "invalid_multiple_unicode" => "invalid token",
        "invalid_surrogate_low" | "invalid_surrogate_high" | "invalid_out_of_range" => {
            "invalid unicode scalar"
        }
        "invalid_escape_unknown" => "unsupported string escape",
        "invalid_escape_nonascii_hex" => "hex string escape must be ASCII",
        "invalid_escape_empty" => "unterminated unicode escape",
        "invalid_escape_too_long" => "unicode escape exceeds six digits",
        "reject_eq_int" | "reject_ne_int" => "equality operands differ",
        "reject_lt_int"
        | "reject_int_gt"
        | "reject_result_int"
        | "reject_parameter_int"
        | "reject_assignment_int" => "expected Char, got Int",
        "reject_add_char" | "reject_sub_char" | "reject_mul_char" | "reject_div_char"
        | "reject_rem_char" | "reject_add_int" => "expected Int(I32), got Char",
        "reject_bitand" => "bitwise operands require identical integer types or Bool",
        "reject_negate" => "unary negation requires a signed integer",
        "reject_not" => "expected Bool, got Char",
        _ => panic!("unexpected rejection: {}: {text}", case.name),
    };
    assert!(
        text.contains(expected),
        "{} expected {expected:?}: {text}",
        case.name
    );
    if case.boundary == Boundary::Type && graph {
        assert!(text.contains("InvalidMetadata"), "{}: {text}", case.name);
    }
}
fn no_compiler_tools(command: &mut Command, scratch: &Scratch) {
    command
        .current_dir(&scratch.0)
        .env_remove("RUSTC_BOOTSTRAP")
        .env("RUSTC", scratch.path("no-rustc"))
        .env("PATH", scratch.path("no-tools"));
}
fn isolated(name: &str, marker: &str, run: fn()) {
    if std::env::var_os(marker).is_some() {
        run();
        return;
    }
    stock_all();
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    no_compiler_tools(&mut command, &scratch);
    command
        .args(["--exact", name, "--nocapture"])
        .env(marker, "1");
    let result = capture(&mut command, &scratch, 240);
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    success(&result, name);
}
fn successful_graph(case: &Case, compiled: &compiler::Compilation, check: bool) {
    let program = frontend::parse(case.source).unwrap();
    frontend::check(&program).unwrap();
    let plan =
        frontend::PlanCtx::from_program(&program, &compiler::ir::PlanBudget::default()).unwrap();
    let expected_bodies = match case.name {
        "params_results" => 4,
        "copy_clone_sized" => 4,
        "unused_char_function" => 3,
        _ => 2,
    };
    let mut source_bodies = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        let universal = match plan.callable_provider(declaration).unwrap() {
            frontend::CallableProvider::Source { .. } => {
                source_bodies += 1;
                plan.callable_env(declaration).unwrap().is_generic()
            }
            frontend::CallableProvider::Recipe { .. } => true,
            frontend::CallableProvider::UnavailableProvided { .. } => panic!("unavailable body"),
        };
        for kind in if universal {
            &[110, 111, 112][..]
        } else {
            &[110][..]
        } {
            let key = Key::new(*kind, vec![index as u64]);
            let nodes: Vec<_> = compiled
                .report
                .node_evaluations
                .iter()
                .filter(|(actual, _)| *actual == key)
                .collect();
            assert_eq!(nodes.len(), 1, "{}: {key:?}", case.name);
            assert_eq!(nodes[0].1, 1);
        }
    }
    assert_eq!(
        source_bodies, expected_bodies,
        "{} body inventory",
        case.name
    );
    assert_eq!(compiled.functions, expected_bodies);
    assert_eq!(compiled.source_bytes, case.bytes);
    assert_eq!(compiled.source_files, 1);
    assert_eq!(compiled.report.failures, 0);
    assert_eq!(compiled.report.events_dropped, 0);
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 110)
            .count(),
        plan.callable_count()
    );
    for kind in [102, 108, 109, 103] {
        let nodes: Vec<_> = compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == kind)
            .collect();
        assert_eq!(
            nodes.len(),
            if kind == 103 && check {
                0
            } else {
                compiled.control_flow.len()
            },
            "{}: kind {kind}",
            case.name
        );
        assert!(nodes.iter().all(|(_, count)| *count == 1));
    }
}
#[test]
fn frozen_chars_stock_and_frontend_boundaries() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 36);
    assert_eq!(
        all.iter().filter(|c| c.boundary == Boundary::Live).count(),
        10
    );
    assert_eq!(
        all.iter()
            .filter(|c| c.boundary == Boundary::Syntax)
            .count(),
        10
    );
    assert_eq!(
        all.iter().filter(|c| c.boundary == Boundary::Type).count(),
        16
    );
    let mut failures = Failures::default();
    for case in all {
        failures.cell(format!("phase {}", case.name), || {
            if case.boundary == Boundary::Syntax {
                rejection(
                    &case,
                    &frontend::parse(case.source)
                        .expect_err("syntax accepted")
                        .to_string(),
                    false,
                );
            } else {
                let parsed = frontend::parse(case.source)
                    .expect("must pass parsing, including type-invalid bodies");
                let checked = frontend::check(&parsed);
                if case.boundary == Boundary::Live {
                    checked.expect("positive type check");
                } else {
                    rejection(
                        &case,
                        &checked.expect_err("type-invalid body accepted").to_string(),
                        false,
                    );
                }
            }
        });
    }
    failures.finish();
}
fn cli_all() {
    let mut failures = Failures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.cell(
                    format!("cli {} workers={workers} check={check}", case.name),
                    || {
                        let scratch = Scratch::new();
                        let input = scratch.source(case.source);
                        let output = scratch.path("native");
                        let mut command = compiler(&input, &output, workers, check);
                        no_compiler_tools(&mut command, &scratch);
                        let result = capture(&mut command, &scratch, 30);
                        if case.boundary == Boundary::Live {
                            success(&result, case.name);
                            let report = String::from_utf8(result.stdout.clone())
                                .expect("successful CLI returns UTF-8 JSON");
                            let compact: String =
                                report.chars().filter(|c| !c.is_whitespace()).collect();
                            assert!(
                                compact.starts_with('{') && compact.ends_with('}'),
                                "{report}"
                            );
                            for field in [
                                "\"capture_complete\":true".to_owned(),
                                "\"failures\":0".to_owned(),
                                format!("\"workers\":{workers}"),
                                format!("\"source_bytes\":{}", case.bytes),
                            ] {
                                assert!(
                                    compact.contains(&format!("{field},"))
                                        || compact.contains(&format!("{field}}}")),
                                    "{report}"
                                );
                            }
                            assert_eq!(
                                compact.contains("\"executable_bytes\":0,")
                                    || compact.contains("\"executable_bytes\":0}"),
                                check,
                                "{report}"
                            );
                            if check {
                                assert!(!output.exists(), "check published executable");
                            } else {
                                runtime(&output, &scratch);
                                runtimes += 1;
                            }
                        } else {
                            assert!(result.stdout.is_empty(), "{result:?}");
                            assert_eq!(result.status.code(), Some(1), "{result:?}");
                            rejection(&case, &String::from_utf8_lossy(&result.stderr), true);
                            assert!(!output.exists(), "invalid source published executable");
                            assert!(!scratch.path("input").exists());
                            assert!(!scratch.path("a.out").exists());
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 144);
    failures.cell("runtime inventory".into(), || assert_eq!(runtimes, 20));
    failures.finish();
}
#[test]
fn chars_cli_workers_one_and_eight_check_build() {
    isolated(
        "chars_cli_workers_one_and_eight_check_build",
        "JR_CHARS_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn chars_observed_graph_workers_one_and_eight_check_build() {
    isolated(
        "chars_observed_graph_workers_one_and_eight_check_build",
        "JR_CHARS_GRAPH_CHILD",
        graph_all,
    );
}
fn graph_all() {
    let mut failures = Failures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.cell(
                    format!("graph {} workers={workers} check={check}", case.name),
                    || {
                        let scratch = Scratch::new();
                        let input = scratch.source(case.source);
                        let result = compiler::compile_file_observed(input, workers, check);
                        if case.boundary == Boundary::Live {
                            let compiled =
                                result.unwrap_or_else(|error| panic!("{}: {error}", case.name));
                            successful_graph(&case, &compiled, check);
                            assert!(compiled.report.capture_complete);
                            assert_eq!(compiled.executable.is_none(), check);
                            assert!(!compiled.control_flow.is_empty());
                            assert_eq!(compiled.control_flow.len(), compiled.analyses.len());
                            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
                            for (index, cfg) in compiled.control_flow.iter().enumerate() {
                                cfg.validate().expect("retained CFG validates");
                                for kind in [102, 108, 109] {
                                    let key = Key::new(kind, vec![index as u64]);
                                    let evaluated: Vec<_> = compiled
                                        .report
                                        .node_evaluations
                                        .iter()
                                        .filter(|(actual, _)| *actual == key)
                                        .collect();
                                    assert_eq!(evaluated.len(), 1, "missing or duplicate {key:?}");
                                    assert_eq!(evaluated[0].1, 1);
                                }
                            }
                            if case.name == "params_results" || case.name == "unused_char_function"
                            {
                                assert!(
                                    compiled
                                        .control_flow
                                        .iter()
                                        .any(|cfg| cfg.result == compiler::ir::Ty::Char),
                                    "char result type lost"
                                );
                            }
                            if check {
                                assert!(
                                    !compiled
                                        .report
                                        .node_evaluations
                                        .iter()
                                        .any(|(key, _)| key.kind == 103)
                                );
                            }
                            if let Some(image) = compiled.executable {
                                let scratch = Scratch::new();
                                let output = scratch.path("graph-native");
                                fs::write(&output, image).unwrap();
                                fs::set_permissions(&output, fs::Permissions::from_mode(0o700))
                                    .unwrap();
                                runtime(&output, &scratch);
                                runtimes += 1;
                            }
                        } else {
                            let failure = result.err().expect("invalid source accepted");
                            rejection(&case, &failure.to_string(), true);
                            let report = failure.report.expect("failure retains graph");
                            assert!(report.capture_complete);
                            assert_eq!(report.events_dropped, 0);
                            assert!(report.failures > 0);
                            assert!(
                                !report
                                    .node_evaluations
                                    .iter()
                                    .any(|(key, _)| key.kind == 103),
                                "rejected source reached codegen/emission"
                            );
                            assert!(!scratch.path("input").exists());
                            assert!(!scratch.path("a.out").exists());
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 144);
    failures.cell("runtime inventory".into(), || assert_eq!(runtimes, 20));
    failures.finish();
}
