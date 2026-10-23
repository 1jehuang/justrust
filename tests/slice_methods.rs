//! Prelude slice methods (`swap`, `to_vec`, `contains`, `first`, `last`,
//! `is_empty`), `Vec::extend_from_slice` and Vec range views `&v[a..b]`.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use oracle::{Case, Expect, check};
use std::process::Command;

#[test]
fn heap_stored_slice_carriers_are_explicitly_unsupported() {
    let cases = [
        (
            "valid",
            "fn main(){let a=[1i32,2];let v:Vec<&[i32]>=a.chunks(1).collect();println!(\"{}\",v.len());}",
            true,
        ),
        (
            "escape",
            "fn main(){let v:Vec<&[i32]>;{let a=[1i32,2];v=a.chunks(1).collect();}println!(\"{}\",v.len());}",
            false,
        ),
    ];
    for (name, source, valid) in cases {
        let dir = scratch(&format!("heap-{name}"));
        let src = dir.join("main.rs");
        std::fs::write(&src, source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&src)
            .arg("-o")
            .arg(dir.join("stock.rmeta"))
            .output()
            .unwrap();
        assert_eq!(
            stock.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&stock.stderr)
        );
        for workers in ["1", "8"] {
            for check in [true, false] {
                let image = dir.join(format!("native-{workers}-{check}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command.arg(&src).args(["--workers", workers]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = command.output().unwrap();
                assert!(!result.status.success());
                assert!(String::from_utf8_lossy(&result.stderr).contains("unsupported: raw storage of slice carriers requires heap lifetime retention"), "{}", String::from_utf8_lossy(&result.stderr));
                assert!(!image.exists());
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

const POSITIVE: &str = r#"
fn f(v: &mut [i32]) { v.swap(0, 1); }
fn merge(v: &[i32]) -> Vec<i32> {
    if v.len() <= 1 { return v.to_vec(); }
    let mid = v.len() / 2;
    let mut out = Vec::new();
    out.extend_from_slice(&v[mid..]);
    out.extend_from_slice(&v[..mid]);
    out
}
fn main() {
    let mut a = vec![1, 2, 3, 4];
    f(a.as_mut_slice());
    let s: &[i32] = a.as_slice();
    let b = s.to_vec();
    let c = s.contains(&2);
    let d = s.contains(&9);
    let e = s.is_empty();
    let fi = s.first();
    let la = s.last();
    let x = b[0];
    println!("{} {} {} {} {:?} {:?}", x, c, d, e, fi, la);
    let t = &a[1..3];
    let n = t.len();
    let u = &a[..2];
    let m = u.to_vec();
    let k = m[1];
    let r = merge(&a[0..4]);
    let r0 = r[0];
    let empty: &[i32] = &a[4..];
    let z = empty.first();
    println!("{} {} {} {} {:?}", n, k, r0, empty.is_empty(), z);
}
"#;
const POSITIVE_OUT: &str = "2 true false false Some(2) Some(4)\n2 1 3 true None\n";

#[test]
fn shared_slice_carriers_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "shared-slice-carriers",
        &[Case {
            name: "reference_tuple_array_and_generic_identity",
            source: r#"
fn indirect(s: &&[u8]) -> usize { (*s).len() }
fn tuple(s: (&[u8], u8)) -> usize { s.0.len() + s.1 as usize }
fn array(s: [&[u8]; 1]) -> usize { s[0].len() }
fn id<T>(x: T) -> T { x }
fn main() {
    let a = [1u8, 2, 3];
    let s: &[u8] = &a;
    let t = id(s);
    println!("{} {} {} {}", indirect(&s), tuple((s, 2)), array([s]), t.len());
}
"#,
            expect: Expect::Run("3 5 3 3\n"),
        }],
    );
}

#[test]
fn slice_methods_match_stock() {
    check(
        "slice-methods",
        &[
            Case {
                name: "methods",
                source: POSITIVE,
                expect: Expect::Run(POSITIVE_OUT),
            },
            Case {
                name: "shared_swap_unused",
                source: "fn bad(s: &[i32]) { s.swap(0, 1); }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "shared_range_mut_unused",
                source: "fn bad(v: &Vec<i32>) { let s = &mut v[1..]; s[0] = 1; }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0596"),
            },
        ],
    );
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let p = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-slice-methods-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn workers_one_and_eight_check_build_run() {
    for workers in ["1", "8"] {
        let dir = scratch(workers);
        let src = dir.join("main.rs");
        std::fs::write(&src, POSITIVE).unwrap();
        let check = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .args(["--check", "--workers", workers])
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        let image = dir.join("native");
        let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("-o")
            .arg(&image)
            .args(["--workers", workers])
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let ran = Command::new(&image).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&ran.stdout), POSITIVE_OUT);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Out-of-bounds `swap` panics with stock's index message under panic=abort.
#[test]
fn swap_out_of_bounds_panics_like_stock() {
    let source = "fn f(v: &mut [i32]) { v.swap(0, 5); }\nfn main() { let mut a = vec![1, 2, 3]; println!(\"before\"); f(a.as_mut_slice()); println!(\"after\"); }\n";
    let dir = scratch("panic");
    let src = dir.join("main.rs");
    std::fs::write(&src, source).unwrap();
    let image = dir.join("native");
    let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .arg(&src)
        .arg("-o")
        .arg(&image)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let native = Command::new(&image).output().unwrap();
    let stock_image = dir.join("stock");
    let stock = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
        .arg(&src)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(stock.status.success());
    let stock = Command::new(&stock_image).output().unwrap();
    assert_eq!(native.stdout, stock.stdout);
    assert_eq!(native.status.code(), stock.status.code());
    let message = "index out of bounds: the len is 3 but the index is 5";
    assert!(String::from_utf8_lossy(&native.stderr).contains(message));
    assert!(String::from_utf8_lossy(&stock.stderr).contains(message));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `[T]::iter` over slice-backed `slice::Iter`, array receivers reaching slice
/// methods through stock's final unsizing probe step, and `Range<int>` as an
/// `Iterator` (corpus 06/11/15 reductions).
#[test]
fn slice_iter_array_unsize_and_range_iterator_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-iter-unsize",
        &[
            Case {
                name: "slice_and_vec_iter",
                source: r#"
fn total(s: &[i64]) -> i64 { let mut t = 0; for x in s.iter() { t += *x; } t }
fn main() {
    let v = vec![1i64, 2, 3];
    let s: &[i64] = &v[1..];
    for x in s.iter() { println!("{}", x); }
    println!("{}", total(v.as_slice()));
    let e: &[i64] = &v[3..];
    let mut it = e.iter();
    println!("{}", it.next().is_none());
}
"#,
                expect: Expect::Run("2\n3\n6\ntrue\n"),
            },
            Case {
                name: "array_receivers_unsize",
                source: r#"
fn main() {
    let a = [4i64, 5, 6];
    let r = &a;
    let n = a.iter().map(|x| *x * 2).count();
    let f = r.iter().fold(0i64, |acc, x| acc + *x);
    println!("{} {} {} {:?}", n, f, a.is_empty(), a.first());
    let m = [[1i64, 2], [3, 4]];
    for row in m.iter() { println!("{}", row.iter().fold(0i64, |s, x| s + *x)); }
}
"#,
                expect: Expect::Run("3 15 false Some(4)\n3\n7\n"),
            },
            Case {
                name: "range_iterator",
                source: r#"
fn main() {
    let a = [10i64, 20, 30];
    let t = (0..3usize).map(|i| a[i]).fold(0i64, |s: i64, x: i64| s + x);
    let mut r = 5..7i64;
    let x = r.next();
    let y = r.next();
    let z = r.next();
    println!("{} {:?} {:?} {:?}", t, x, y, z);
    println!("{}", (3u8..3).count());
}
"#,
                expect: Expect::Run("60 Some(5) Some(6) None\n0\n"),
            },
            Case {
                name: "empty_and_zero_sized_iteration",
                source: "fn main(){let empty:[i32;0]=[];let units=[();3];println!(\"{} {}\",empty.iter().count(),units.iter().count());}",
                expect: Expect::Run("0 3\n"),
            },
            Case {
                name: "iterator_keeps_owner_borrowed",
                source: "fn main(){let mut v=vec![1i32,2];let mut it=v.iter();v.push(3);println!(\"{:?}\",it.next());}",
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "iterator_cannot_escape_owner",
                source: "fn main(){let mut it={let v=vec![1i32,2];v.iter()};println!(\"{:?}\",it.next());}",
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "array_iter_item_mismatch_unused",
                source: "fn bad() { let a = [1i64, 2]; let _: i64 = a.iter().count(); }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "range_item_mismatch_unused",
                source: "fn bad() { let mut r = 0..2u8; let _: Option<i64> = r.next(); }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
#[ignore = "pending general heap-stored reference lifetime authority; not supported by chunks milestone"]
fn collected_slice_references_require_heap_lifetime_authority() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "heap-slice-retention",
        &[
            Case {
                name: "collected_slices",
                source: r#"fn main(){let a=[1i32,2,3,4];let saved:Vec<&[i32]>=a.chunks(3).collect();println!("{} {}",saved[0][2],saved[1][0]);}"#,
                expect: Expect::Run("3 4\n"),
            },
            Case {
                name: "collected_chunks_keep_owner_borrowed",
                source: r#"fn main(){let mut v=vec![1i32,2];let saved:Vec<&[i32]>=v.chunks(1).collect();v.push(3);println!("{}",saved[0][0]);}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "collected_chunk_outlives_owner",
                source: r#"fn main(){let c;{let v=vec![1i32,2];let saved:Vec<&[i32]>=v.chunks(1).collect();c=saved[0];}println!("{}",c[0]);}"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn slice_chunks_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-chunks",
        &[
            Case {
                name: "chunk_sums",
                source: r#"fn main(){let v=vec![1i32,2,3,4,5,6,7];let sums:Vec<i32>=v.chunks(3).map(|c|c.iter().sum::<i32>()).collect();println!("{:?}",sums);let a=[1i32,2,3,4,5];let mut c=a.chunks(2);println!("{:?} {:?} {:?} {}",c.next_back().unwrap(),c.next().unwrap(),c.next_back().unwrap(),c.next().is_none());}"#,
                expect: Expect::Run(
                    "[6, 15, 7]
[5] [1, 2] [3, 4] true
",
                ),
            },
            Case {
                name: "empty_huge_and_zst",
                source: r#"fn main(){let a:[i32;0]=[];let b=[1i32,2];let z=[();5];println!("{} {} {}",a.chunks(usize::MAX).count(),b.chunks(usize::MAX).next().unwrap().len(),z.chunks(2).rev().map(|c|c.len()).sum::<usize>());}"#,
                expect: Expect::Run(
                    "0 2 5
",
                ),
            },
            Case {
                name: "nested_shared_descriptors",
                source: r#"fn split(s:&[i32])->(Option<&[i32]>,&[i32]){(Some(&s[..2]),&s[2..])}fn main(){let a=[1i32,2,3,4];let (x,y)=split(&a);println!("{:?} {:?}",x.unwrap(),y);}"#,
                expect: Expect::Run("[1, 2] [3, 4]\n"),
            },
            Case {
                name: "chunks_keep_owner_borrowed",
                source: r#"fn main(){let mut v=vec![1i32,2];let mut c=v.chunks(1);v.push(3);println!("{:?}",c.next());}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "chunk_outlives_owner",
                source: r#"fn main(){let c;{let v=vec![1i32,2];c=v.chunks(1).next().unwrap();}println!("{:?}",c);}"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn zero_chunk_panics_like_stock() {
    let source =
        "fn main(){let a=[1i32,2];println!(\"before\");let _=a.chunks(0);println!(\"after\");}";
    let dir = scratch("zero-chunk");
    let src = dir.join("main.rs");
    std::fs::write(&src, source).unwrap();
    let stock = dir.join("stock");
    let built = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(&src)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    let expected = Command::new(stock).output().unwrap();
    assert!(!expected.status.success());
    assert_eq!(expected.stdout, b"before\n");
    for workers in ["1", "8"] {
        let check = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .args(["--check", "--workers", workers])
            .output()
            .unwrap();
        assert!(check.status.success(), "{check:?}");
        let image = dir.join(format!("native-{workers}"));
        let built = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("-o")
            .arg(&image)
            .args(["--workers", workers])
            .output()
            .unwrap();
        assert!(built.status.success(), "{built:?}");
        let actual = Command::new(image).output().unwrap();
        assert!(!actual.status.success());
        assert_eq!(actual.stdout, expected.stdout);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn double_ended_slice_iteration_matches_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-double-ended",
        &[
            Case {
                name: "mixed_front_back_and_reverse",
                source: r#"fn main(){let a=[1i32,2,3,4];let mut it=a.iter();println!("{} {} {} {} {}",it.next_back().unwrap(),it.next().unwrap(),it.next_back().unwrap(),it.next().unwrap(),it.next_back().is_none());let v:Vec<i32>=a.iter().rev().copied().collect();println!("{} {}",v[0],v[3]);}"#,
                expect: Expect::Run("4 1 3 2 true\n4 1\n"),
            },
            Case {
                name: "empty_singleton_and_zero_sized",
                source: r#"fn main(){let empty:[i32;0]=[];let mut e=empty.iter();let one=[7i32];let mut o=one.iter();let units=[();3];let mut z=units.iter();println!("{} {} {} {} {} {} {}",e.next_back().is_none(),o.next_back().unwrap(),o.next().is_none(),z.next_back().is_some(),z.next().is_some(),z.next_back().is_some(),z.next().is_none());}"#,
                expect: Expect::Run("true 7 true true true true true\n"),
            },
            Case {
                name: "reverse_keeps_owner_borrowed",
                source: r#"fn main(){let mut v=vec![1i32,2];let mut it=v.iter().rev();v.push(3);println!("{:?}",it.next());}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "reverse_cannot_escape_owner",
                source: r#"fn main(){let mut it={let v=vec![1i32,2];v.iter().rev()};println!("{:?}",it.next());}"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn inclusive_and_reverse_integer_ranges_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "inclusive-integer-ranges",
        &[
            Case {
                name: "inclusive_i8",
                source: "fn main(){ let mut top=(i8::MAX-1i8)..=i8::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i8::MAX-1i8,b==i8::MAX,top.next().is_none()); let mut low=(i8::MIN..=i8::MIN+1i8).rev(); println!(\"{} {} {}\",low.next().unwrap()==i8::MIN+1i8,low.next().unwrap()==i8::MIN,low.next().is_none()); let v:Vec<i8>=(1i8..=3i8).collect(); let w:Vec<i8>=(1i8..4i8).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i8..=2i8).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_i16",
                source: "fn main(){ let mut top=(i16::MAX-1i16)..=i16::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i16::MAX-1i16,b==i16::MAX,top.next().is_none()); let mut low=(i16::MIN..=i16::MIN+1i16).rev(); println!(\"{} {} {}\",low.next().unwrap()==i16::MIN+1i16,low.next().unwrap()==i16::MIN,low.next().is_none()); let v:Vec<i16>=(1i16..=3i16).collect(); let w:Vec<i16>=(1i16..4i16).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i16..=2i16).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_i32",
                source: "fn main(){ let mut top=(i32::MAX-1i32)..=i32::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i32::MAX-1i32,b==i32::MAX,top.next().is_none()); let mut low=(i32::MIN..=i32::MIN+1i32).rev(); println!(\"{} {} {}\",low.next().unwrap()==i32::MIN+1i32,low.next().unwrap()==i32::MIN,low.next().is_none()); let v:Vec<i32>=(1i32..=3i32).collect(); let w:Vec<i32>=(1i32..4i32).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i32..=2i32).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_i64",
                source: "fn main(){ let mut top=(i64::MAX-1i64)..=i64::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i64::MAX-1i64,b==i64::MAX,top.next().is_none()); let mut low=(i64::MIN..=i64::MIN+1i64).rev(); println!(\"{} {} {}\",low.next().unwrap()==i64::MIN+1i64,low.next().unwrap()==i64::MIN,low.next().is_none()); let v:Vec<i64>=(1i64..=3i64).collect(); let w:Vec<i64>=(1i64..4i64).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i64..=2i64).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_i128",
                source: "fn main(){ let mut top=(i128::MAX-1i128)..=i128::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i128::MAX-1i128,b==i128::MAX,top.next().is_none()); let mut low=(i128::MIN..=i128::MIN+1i128).rev(); println!(\"{} {} {}\",low.next().unwrap()==i128::MIN+1i128,low.next().unwrap()==i128::MIN,low.next().is_none()); let v:Vec<i128>=(1i128..=3i128).collect(); let w:Vec<i128>=(1i128..4i128).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i128..=2i128).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_isize",
                source: "fn main(){ let mut top=(isize::MAX-1isize)..=isize::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==isize::MAX-1isize,b==isize::MAX,top.next().is_none()); let mut low=(isize::MIN..=isize::MIN+1isize).rev(); println!(\"{} {} {}\",low.next().unwrap()==isize::MIN+1isize,low.next().unwrap()==isize::MIN,low.next().is_none()); let v:Vec<isize>=(1isize..=3isize).collect(); let w:Vec<isize>=(1isize..4isize).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3isize..=2isize).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u8",
                source: "fn main(){ let mut top=(u8::MAX-1u8)..=u8::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u8::MAX-1u8,b==u8::MAX,top.next().is_none()); let mut low=(u8::MIN..=u8::MIN+1u8).rev(); println!(\"{} {} {}\",low.next().unwrap()==u8::MIN+1u8,low.next().unwrap()==u8::MIN,low.next().is_none()); let v:Vec<u8>=(1u8..=3u8).collect(); let w:Vec<u8>=(1u8..4u8).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u8..=2u8).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u16",
                source: "fn main(){ let mut top=(u16::MAX-1u16)..=u16::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u16::MAX-1u16,b==u16::MAX,top.next().is_none()); let mut low=(u16::MIN..=u16::MIN+1u16).rev(); println!(\"{} {} {}\",low.next().unwrap()==u16::MIN+1u16,low.next().unwrap()==u16::MIN,low.next().is_none()); let v:Vec<u16>=(1u16..=3u16).collect(); let w:Vec<u16>=(1u16..4u16).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u16..=2u16).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u32",
                source: "fn main(){ let mut top=(u32::MAX-1u32)..=u32::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u32::MAX-1u32,b==u32::MAX,top.next().is_none()); let mut low=(u32::MIN..=u32::MIN+1u32).rev(); println!(\"{} {} {}\",low.next().unwrap()==u32::MIN+1u32,low.next().unwrap()==u32::MIN,low.next().is_none()); let v:Vec<u32>=(1u32..=3u32).collect(); let w:Vec<u32>=(1u32..4u32).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u32..=2u32).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u64",
                source: "fn main(){ let mut top=(u64::MAX-1u64)..=u64::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u64::MAX-1u64,b==u64::MAX,top.next().is_none()); let mut low=(u64::MIN..=u64::MIN+1u64).rev(); println!(\"{} {} {}\",low.next().unwrap()==u64::MIN+1u64,low.next().unwrap()==u64::MIN,low.next().is_none()); let v:Vec<u64>=(1u64..=3u64).collect(); let w:Vec<u64>=(1u64..4u64).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u64..=2u64).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u128",
                source: "fn main(){ let mut top=(u128::MAX-1u128)..=u128::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u128::MAX-1u128,b==u128::MAX,top.next().is_none()); let mut low=(u128::MIN..=u128::MIN+1u128).rev(); println!(\"{} {} {}\",low.next().unwrap()==u128::MIN+1u128,low.next().unwrap()==u128::MIN,low.next().is_none()); let v:Vec<u128>=(1u128..=3u128).collect(); let w:Vec<u128>=(1u128..4u128).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u128..=2u128).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_usize",
                source: "fn main(){ let mut top=(usize::MAX-1usize)..=usize::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==usize::MAX-1usize,b==usize::MAX,top.next().is_none()); let mut low=(usize::MIN..=usize::MIN+1usize).rev(); println!(\"{} {} {}\",low.next().unwrap()==usize::MIN+1usize,low.next().unwrap()==usize::MIN,low.next().is_none()); let v:Vec<usize>=(1usize..=3usize).collect(); let w:Vec<usize>=(1usize..4usize).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3usize..=2usize).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "wrong_inclusive_item_unused",
                source: "fn bad(){let mut r=1u8..=2;let _:Option<i32>=r.next();}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
