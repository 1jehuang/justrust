//! Stock differential coverage for `Iterator::fold`, a prelude default method.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn contextual_numeric_range_receivers_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "contextual-ranges",
        &[
            Case {
                name: "signed_collection",
                source: r#"fn main(){let values:Vec<i32>=(1..=4).collect();println!("{:?}",values);}"#,
                expect: Expect::Run("[1, 2, 3, 4]\n"),
            },
            Case {
                name: "unsigned_collection",
                source: r#"fn main(){let values:Vec<u64>=(1..4).collect();println!("{:?}",values);}"#,
                expect: Expect::Run("[1, 2, 3]\n"),
            },
            Case {
                name: "nested_scope_receivers",
                source: r#"fn main(){let a:Vec<i32>=(1..3).collect();{let b:Vec<u64>=(3..5).collect();println!("{:?}",b);}let c:Vec<i32>=(5..7).collect();println!("{:?} {:?}",a,c);}"#,
                expect: Expect::Run("[3, 4]\n[1, 2] [5, 6]\n"),
            },
            Case {
                name: "contextual_literal_overflow",
                source: r#"fn unused(){let _:Vec<u8>=(0..300).collect();}fn main(){}"#,
                expect: Expect::Invalid("out of range"),
            },
            Case {
                name: "explicit_type_mismatch",
                source: r#"fn unused(){let _:Vec<u64>=(1i32..4i32).collect();}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn iterator_zip_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "zip",
        &[
            Case {
                name: "conversion_at_construction",
                source: r#"struct Bag; impl IntoIterator for Bag {type Item=i32;type IntoIter=std::ops::Range<i32>;fn into_iter(self)->Self::IntoIter{println!("converted");1i32..3i32}} fn main(){let z=(5i32..8i32).zip(Bag);println!("created");let v:Vec<(i32,i32)>=z.collect();println!("{:?}",v);}"#,
                expect: Expect::Run("converted\ncreated\n[(5, 1), (6, 2)]\n"),
            },
            Case {
                name: "wrong_projected_item",
                source: r#"fn main(){let _:Vec<(i32,bool)>=(1i32..4i32).zip("ab".chars()).collect();}"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "borrowed_zip_escape",
                source: r#"fn main(){let z;{let values=vec![1i32,2i32];z=(0i32..2i32).zip(values.iter());}println!("{:?}",z.collect::<Vec<_>>());}"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "unequal_lengths",
                source: r#"fn main(){let v:Vec<(i32,char)>=(1i32..5i32).zip("abc".chars()).collect();println!("{:?}",v);let w:Vec<(i32,i32)>=(0i32..0i32).zip(1i32..3i32).collect();println!("{}",w.len());}"#,
                expect: Expect::Run(
                    "[(1, 'a'), (2, 'b'), (3, 'c')]
0
",
                ),
            },
            Case {
                name: "poll_order",
                source: r#"struct P{n:i32,id:i32} impl Iterator for P{type Item=i32;fn next(&mut self)->Option<i32>{self.n+=1;println!("{}:{}",self.id,self.n);if self.n==2{None}else{Some(self.n)}}} struct Source; impl IntoIterator for Source{type Item=i32;type IntoIter=P;fn into_iter(self)->P{P{n:0,id:2}}} fn main(){let mut z=P{n:0,id:1}.zip(Source);println!("{:?}",z.next());println!("{:?}",z.next());println!("{:?}",z.next());println!("{:?}",z.next());}"#,
                expect: Expect::Run(
                    "1:1
2:1
Some((1, 1))
1:2
None
1:3
2:2
None
1:4
2:3
Some((4, 3))
",
                ),
            },
        ],
    );
}

#[test]
fn iterator_bounded_adapters_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "bounded-adapters",
        &[
            Case {
                name: "chained_values",
                source: r#"fn main(){let v:Vec<i32>=(0i32..10i32).rev().skip(2).step_by(3).take(3).collect();println!("{} {} {} {}",v.len(),v[0],v[1],v[2]);println!("{} {} {}",(0i32..5i32).skip(99).count(),(0i32..5i32).take(0).count(),(0i32..5i32).step_by(1).count());}"#,
                expect: Expect::Run("3 7 4 1\n0 0 5\n"),
            },
            Case {
                name: "non_fused_inputs",
                source: r#"struct Pulse{n:i32}impl Iterator for Pulse{type Item=i32;fn next(&mut self)->Option<i32>{self.n+=1;if self.n==2||self.n==4{None}else{Some(self.n)}}}fn main(){let mut t=Pulse{n:0}.take(3);println!("{:?} {:?} {:?} {:?}",t.next(),t.next(),t.next(),t.next());let mut s=Pulse{n:0}.skip(1);println!("{:?} {:?} {:?} {:?}",s.next(),s.next(),s.next(),s.next());let mut p=Pulse{n:0}.step_by(2);println!("{:?} {:?} {:?} {:?}",p.next(),p.next(),p.next(),p.next());}"#,
                expect: Expect::Run(
                    "Some(1) None Some(3) None\nNone Some(3) None Some(5)\nSome(1) None None Some(6)\n",
                ),
            },
            Case {
                name: "zero_take_and_large_steps",
                source: r#"struct Probe;impl Iterator for Probe{type Item=i32;fn next(&mut self)->Option<i32>{println!("polled");Some(1)}}fn main(){let mut p=Probe.take(0);println!("{:?}",p.next());println!("{} {}",(0i32..0i32).skip(usize::MAX).count(),(0i32..1i32).step_by(usize::MAX).count());}"#,
                expect: Expect::Run("None\n0 1\n"),
            },
            Case {
                name: "bounded_borrow_cannot_escape",
                source: r#"fn main(){let mut it={let v=vec![1i32,2];v.iter().skip(1).take(1)};println!("{:?}",it.next());}"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn zero_step_panics_like_stock() {
    use std::os::unix::fs::PermissionsExt;
    let source =
        r#"fn main(){println!("before");let _=(0i32..3i32).step_by(0);println!("after");}"#;
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-zero-step-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source_path = root.join("main.rs");
    std::fs::write(&source_path, source).unwrap();
    let stock_image = root.join("stock");
    let stock = std::process::Command::new(
        std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into()),
    )
    .env_remove("RUSTC_BOOTSTRAP")
    .args(["--edition=2024", "-Cpanic=abort"])
    .arg(&source_path)
    .arg("-o")
    .arg(&stock_image)
    .output()
    .unwrap();
    assert!(stock.status.success(), "{stock:?}");
    let stock = std::process::Command::new(stock_image).output().unwrap();
    assert!(!stock.status.success());
    assert_eq!(stock.stdout, b"before\n");
    for workers in [1, 8] {
        let checked = justrust_engine::compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        let compiled = justrust_engine::compiler::compile(source.into(), workers, false).unwrap();
        let image = root.join(format!("native-{workers}"));
        std::fs::write(&image, compiled.executable.unwrap()).unwrap();
        std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o700)).unwrap();
        let native = std::process::Command::new(image).output().unwrap();
        assert!(!native.status.success());
        assert_eq!(native.stdout, stock.stdout);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn custom_iterator_product_matches_stock() {
    closure_oracle::check(
        "custom-product",
        &[closure_oracle::Case {
            name: "custom_result_and_zero",
            source: r#"struct Total(i32);impl std::iter::Product<i32> for Total {fn product<I:Iterator<Item=i32>>(iter:I)->Self{Total(iter.fold(1,|a,b|a*b))}}fn main(){let p:Total=(2i32..5i32).product();let q:i32=(-2i32..1i32).product();println!("{} {}",p.0,q);}"#,
            expect: closure_oracle::Expect::Run("24 0\n"),
        }],
    );
}

#[test]
fn iterator_product_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-product",
        &[
            Case {
                name: "product_i8",
                source: r#"fn main(){let a=[2i8,3i8,4i8];let p:i8=a.iter().product();let q:i8=(1i8..=4i8).product();let e:i8=(1i8..1i8).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_i16",
                source: r#"fn main(){let a=[2i16,3i16,4i16];let p:i16=a.iter().product();let q:i16=(1i16..=4i16).product();let e:i16=(1i16..1i16).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_i32",
                source: r#"fn main(){let a=[2i32,3i32,4i32];let p:i32=a.iter().product();let q:i32=(1i32..=4i32).product();let e:i32=(1i32..1i32).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_i64",
                source: r#"fn main(){let a=[2i64,3i64,4i64];let p:i64=a.iter().product();let q:i64=(1i64..=4i64).product();let e:i64=(1i64..1i64).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_i128",
                source: r#"fn main(){let a=[2i128,3i128,4i128];let p:i128=a.iter().product();let q:i128=(1i128..=4i128).product();let e:i128=(1i128..1i128).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_isize",
                source: r#"fn main(){let a=[2isize,3isize,4isize];let p:isize=a.iter().product();let q:isize=(1isize..=4isize).product();let e:isize=(1isize..1isize).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_u8",
                source: r#"fn main(){let a=[2u8,3u8,4u8];let p:u8=a.iter().product();let q:u8=(1u8..=4u8).product();let e:u8=(1u8..1u8).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_u16",
                source: r#"fn main(){let a=[2u16,3u16,4u16];let p:u16=a.iter().product();let q:u16=(1u16..=4u16).product();let e:u16=(1u16..1u16).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_u32",
                source: r#"fn main(){let a=[2u32,3u32,4u32];let p:u32=a.iter().product();let q:u32=(1u32..=4u32).product();let e:u32=(1u32..1u32).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_u64",
                source: r#"fn main(){let a=[2u64,3u64,4u64];let p:u64=a.iter().product();let q:u64=(1u64..=4u64).product();let e:u64=(1u64..1u64).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_u128",
                source: r#"fn main(){let a=[2u128,3u128,4u128];let p:u128=a.iter().product();let q:u128=(1u128..=4u128).product();let e:u128=(1u128..1u128).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "product_usize",
                source: r#"fn main(){let a=[2usize,3usize,4usize];let p:usize=a.iter().product();let q:usize=(1usize..=4usize).product();let e:usize=(1usize..1usize).product();println!("{} {} {}",p,q,e);}"#,
                expect: Expect::Run("24 24 1\n"),
            },
            Case {
                name: "mapped_product",
                source: r#"fn main(){let p:i64=(1i32..=5i32).map(|x|x as i64).product();println!("{}",p);}"#,
                expect: Expect::Run("120\n"),
            },
            Case {
                name: "missing_product_impl",
                source: r#"struct Nope;fn bad(){let _:Nope=(1i32..4i32).product();}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn unchanged_matrix_matches_stock_at_both_worker_counts() {
    closure_oracle::check(
        "unchanged-matrix",
        &[closure_oracle::Case {
            name: "full_matrix_program",
            source: include_str!("fixtures/full_programs/06_matrix.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "fixtures/full_programs/06_matrix.stdout"
            )),
        }],
    );
}

#[test]
fn iterator_filter_inferred_char_matches_stock() {
    closure_oracle::check(
        "iterator-filter-inferred-char",
        &[
            closure_oracle::Case {
                name: "inferred_char_filter_map",
                source: r#"fn main(){let cleaned:Vec<char>="Ab 1!".chars().filter(|c|c.is_alphanumeric()).map(|c|c.to_ascii_lowercase()).collect();println!("{} {} {}",cleaned[0],cleaned[1],cleaned[2]);}"#,
                expect: closure_oracle::Expect::Run("a b 1\n"),
            },
            closure_oracle::Case {
                name: "nested_inferred_filters",
                source: r#"fn main(){let cleaned:Vec<char>="Ab 1!".chars().filter(|c|c.is_alphanumeric()).filter(|c|*c!='1').map(|c|c.to_ascii_lowercase()).collect();println!("{} {}",cleaned[0],cleaned[1]);}"#,
                expect: closure_oracle::Expect::Run("a b\n"),
            },
            closure_oracle::Case {
                name: "invalid_filter_result_unused",
                source: r#"fn bad(){let cleaned:Vec<char>="Ab".chars().filter(|c|1).map(|c|c.to_ascii_lowercase()).collect();}fn main(){}"#,
                expect: closure_oracle::Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn unchanged_string_processing_matches_stock_at_both_worker_counts() {
    closure_oracle::check(
        "unchanged-string-processing",
        &[closure_oracle::Case {
            name: "full_string_program",
            source: include_str!("fixtures/full_programs/10_string_processing.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "fixtures/full_programs/10_string_processing.stdout"
            )),
        }],
    );
}

#[test]
fn reverse_characters_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "reverse-characters",
        &[
            Case {
                name: "unicode_and_empty_reverse",
                source: r#"fn main(){let text:String="aé🦀".chars().rev().collect();let empty:String="".chars().rev().collect();println!("{} {}",text,empty.len());}"#,
                expect: Expect::Run("🦀éa 0\n"),
            },
            Case {
                name: "interleaved_character_ends",
                source: r#"fn main(){let mut c="aé🦀z".chars();println!("{:?} {:?} {:?} {:?} {:?}",c.next(),c.next_back(),c.next_back(),c.next(),c.next_back());}"#,
                expect: Expect::Run("Some('a') Some('z') Some('🦀') Some('é') None\n"),
            },
            Case {
                name: "reverse_requires_double_ended",
                source: "struct Forward;impl Iterator for Forward{type Item=i32;fn next(&mut self)->Option<i32>{None}}fn bad(){let x=Forward.rev();}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn contextual_iterator_trait_arguments_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "contextual-trait-arguments",
        &[
            Case {
                name: "chunk_sum_and_product_from_collection_context",
                source: "fn main(){let a=[1i32,2,3,4,5];let sums:Vec<i32>=a.chunks(2).map(|c|c.iter().sum()).collect();let products:Vec<i32>=a.chunks(2).map(|c|c.iter().product()).collect();println!(\"{:?} {:?}\",sums,products);}",
                expect: Expect::Run("[3, 7, 5] [2, 12, 5]\n"),
            },
            Case {
                name: "custom_generic_collection_context",
                source: "use std::iter::FromIterator;struct Batch<T>{values:Vec<T>}impl<T> FromIterator<T> for Batch<T>{fn from_iter<I:IntoIterator<Item=T>>(source:I)->Self{let mut iter=source.into_iter();let mut values=Vec::new();loop{match iter.next(){Some(value)=>values.push(value),None=>return Batch{values},}}}}fn main(){let a=[1i32,2,3];let b:Batch<i32>=a.chunks(2).map(|c|c.iter().sum()).collect();println!(\"{:?}\",b.values);}",
                expect: Expect::Run("[3, 3]\n"),
            },
            Case {
                name: "wrong_context_does_not_authorize_sum",
                source: "fn bad(){let a=[1i32,2];let result:Vec<u64>=a.chunks(1).map(|c|c.iter().sum()).collect();}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn iterator_sum_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-sum",
        &[
            Case {
                name: "mapped_integer",
                source: "fn main(){let a=[1i64,2,3]; let n:i64=(0usize..3usize).map(|i|a[i]).sum();println!(\"{}\",n);}",
                expect: Expect::Run("6\n"),
            },
            Case {
                name: "borrowed_and_empty",
                source: "fn main(){let a=[2i32,3,4];let n:i32=a.iter().sum();let empty:[i32;0]=[];let z:i32=empty.iter().sum();println!(\"{} {}\",n,z);}",
                expect: Expect::Run("9 0\n"),
            },
            Case {
                name: "signed_widths",
                source: "fn main(){let a=[1i8,2];let b=[1i16,2];let c=[1i128,2];let d=[1isize,2];println!(\"{} {} {} {}\",a.iter().copied().sum::<i8>(),b.iter().sum::<i16>(),c.iter().copied().sum::<i128>(),d.iter().sum::<isize>());}",
                expect: Expect::Run("3 3 3 3\n"),
            },
            Case {
                name: "unsigned_widths",
                source: "fn main(){let a=[1u8,2];let b=[1u16,2];let c=[1u32,2];let d=[1u64,2];let e=[1u128,2];let f=[1usize,2];println!(\"{} {} {} {} {} {}\",a.iter().sum::<u8>(),b.iter().copied().sum::<u16>(),c.iter().sum::<u32>(),d.iter().copied().sum::<u64>(),e.iter().sum::<u128>(),f.iter().copied().sum::<usize>());}",
                expect: Expect::Run("3 3 3 3 3 3\n"),
            },
            Case {
                name: "user_defined_sum",
                source: "use std::iter::Sum;struct Total{value:i32}impl Sum<i32> for Total{fn sum<I:Iterator<Item=i32>>(mut iter:I)->Self{let mut result=Total{value:10};loop{match iter.next(){Some(n)=>result.value+=n,None=>return result,}}}}fn main(){let result:Total=(0i32..4i32).sum();println!(\"{}\",result.value);}",
                expect: Expect::Run("16\n"),
            },
            Case {
                name: "wrong_sum_result_unused",
                source: "fn bad(){let a=[1i32];let n:u64=a.iter().sum();}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn iterator_collect_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-collect",
        &[
            Case {
                name: "character_vector",
                source: r#"fn main() { let chars: Vec<char> = "aé🦀".chars().collect(); println!("{} {} {} {}", chars.len(), chars[0], chars[1], chars[2]); }"#,
                expect: Expect::Run("3 a é 🦀\n"),
            },
            Case {
                name: "string_characters",
                source: r#"fn main() { let text: String = "aé🦀".chars().collect(); println!("{}", text); }"#,
                expect: Expect::Run("aé🦀\n"),
            },
            Case {
                name: "string_borrowed_characters",
                source: r#"fn main() { let chars = ['a', 'é', '🦀']; let text: String = chars.iter().collect(); println!("{}", text); }"#,
                expect: Expect::Run("aé🦀\n"),
            },
            Case {
                name: "empty_collection",
                source: r#"fn main() { let values: Vec<char> = "".chars().collect(); let text: String = "".chars().collect(); println!("{} {}", values.len(), text.len()); }"#,
                expect: Expect::Run("0 0\n"),
            },
            Case {
                name: "user_defined_iterator",
                source: r#"struct Count { n: i32 } impl Iterator for Count { type Item = i32; fn next(&mut self) -> Option<i32> { if self.n == 3 { None } else { let n = self.n; self.n += 1; Some(n) } } } fn main() { let values: Vec<i32> = Count { n: 0 }.collect(); println!("{} {}", values.len(), values[2]); }"#,
                expect: Expect::Run("3 2\n"),
            },
            Case {
                name: "wrong_element_type",
                source: r#"fn main() { let values: Vec<bool> = (0i32..3i32).collect(); }"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn iterator_collect_mapped_range_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-collect-map",
        &[Case {
            name: "range_map",
            source: r#"fn main() { let values: Vec<i32> = (0i32..4i32).map(|x| x * x).collect(); println!("{} {} {}", values.len(), values[1], values[3]); }"#,
            expect: Expect::Run("4 1 9\n"),
        }],
    );
}

#[test]
fn iterator_collect_mapped_strings_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-collect-mapped-strings",
        &[Case {
            name: "borrowed_integer_strings",
            source: r#"fn main() { let row = [1i64, 2i64]; let parts: Vec<String> = row.iter().map(|x| x.to_string()).collect(); println!("[{}]", parts.join(", ")); }"#,
            expect: Expect::Run("[1, 2]\n"),
        }],
    );
}

#[test]
fn generic_method_owner_projections_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "method-owner-projections",
        &[
            Case {
                name: "projected_parameter_and_result",
                source: r#"
trait Source {
    type Item;
    fn choose<U>(&self, value: Self::Item, tag: U) -> Option<Self::Item>;
}
struct Numbers;
impl Source for Numbers {
    type Item = i32;
    fn choose<U>(&self, value: i32, tag: U) -> Option<i32> { Some(value) }
}
fn main() {
    let source = Numbers;
    let chosen: Option<i32> = source.choose(7i32, true);
    println!("{:?}", chosen);
}
"#,
                expect: Expect::Run("Some(7)\n"),
            },
            Case {
                name: "generic_owner_and_method_binders",
                source: r#"
trait Source { type Item; fn choose<U>(&self, value: Self::Item, tag: U) -> Self::Item; }
struct Holder<T> { value: T }
impl<T> Source for Holder<T> {
    type Item = T;
    fn choose<U>(&self, value: T, tag: U) -> T { value }
}
fn main() {
    let source = Holder { value: 1i32 };
    println!("{}", source.choose(9i32, false));
}
"#,
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "wrong_projected_implementation_unused",
                source: r#"
trait Source { type Item; fn choose<U>(&self, value: Self::Item, tag: U) -> Self::Item; }
struct Numbers;
impl Source for Numbers {
    type Item = i32;
    fn choose<U>(&self, value: u8, tag: U) -> i32 { 7 }
}
fn main() {}
"#,
                expect: Expect::Invalid("E0053"),
            },
            Case {
                name: "projected_result_mismatch_unused",
                source: r#"
trait Source {
    type Item;
    fn choose<U>(&self, tag: U) -> Option<Self::Item>;
}
struct Numbers;
impl Source for Numbers {
    type Item = i32;
    fn choose<U>(&self, tag: U) -> Option<i32> { Some(7) }
}
fn bad() { let chosen: Option<bool> = Numbers.choose(true); }
fn main() {}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn iterator_fold_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-fold",
        &[
            Case {
                name: "sums_borrowed_items",
                source: r#"
fn main() {
    let v = vec![1i64, 2, 3, 4];
    let s = v.iter().fold(0i64, |a, x| a + *x);
    println!("{}", s);
}
"#,
                expect: Expect::Run("10\n"),
            },
            Case {
                name: "accumulator_type_differs_from_item_and_empty_returns_init",
                source: r#"
fn main() {
    let v = vec![3u8, 4, 5];
    let n = v.iter().fold(0u64, |acc, x| acc * 10 + (*x as u64));
    println!("{}", n);
    let e: Vec<u8> = Vec::new();
    println!("{}", e.iter().fold(7u32, |a, _| a + 1));
}
"#,
                expect: Expect::Run("345\n7\n"),
            },
            Case {
                name: "folds_left_to_right",
                source: r#"
fn main() {
    let v = vec![1i64, 2, 3];
    println!("{}", v.iter().fold(100i64, |a, x| a - *x * 10));
    println!("{}", v.iter().fold(0i64, |a, x| a * 2 + *x));
}
"#,
                expect: Expect::Run("40\n11\n"),
            },
            // A graph-realized nominal accumulator: the closure body shares the
            // parent's nominal identity authority (corpus 19 reduction).
            Case {
                name: "nominal_accumulator_with_derived_debug",
                source: r#"
#[derive(Clone, Copy, Debug)]
struct V { x: i32 }
fn main() {
    let v = vec![V { x: 1 }, V { x: 2 }];
    let s = v.iter().fold(V { x: 0 }, |acc, &p| V { x: acc.x + p.x });
    println!("{:?}", s);
}
"#,
                expect: Expect::Run("V { x: 3 }\n"),
            },
            // A user `Add` impl called inside the closure body: the realized
            // nominal call is typed in the closure's own arena (corpus 19).
            Case {
                name: "nominal_operator_trait_call_in_closure_body",
                source: r#"
use std::ops::Add;
#[derive(Clone, Copy, Debug)]
struct V { x: i32 }
impl Add for V { type Output = V; fn add(self, o: V) -> V { V { x: self.x + o.x } } }
fn main() {
    let v = vec![V { x: 1 }, V { x: 2 }];
    let s = v.iter().fold(V { x: 0 }, |acc, &p| acc + p);
    println!("{:?}", s);
}
"#,
                expect: Expect::Run("V { x: 3 }\n"),
            },
            Case {
                name: "nominal_operator_without_impl_in_closure_body_rejects",
                source: r#"
#[derive(Clone, Copy, Debug)]
struct V { x: i32 }
fn main() {
    let v = vec![V { x: 1 }];
    let s = v.iter().fold(V { x: 0 }, |acc, &p| acc + p);
    println!("{:?}", s);
}
"#,
                expect: Expect::Invalid(""),
            },
            // A by-value `mut acc` closure parameter that calls `&mut self`
            // methods (`|mut acc, x| { acc.push(..); acc }`) is rejected by
            // the current checker ("canonical method receiver formal
            // mismatch"), independent of fold. Tracked in SEMANTIC-CORE.md.
            Case {
                name: "closure_must_return_accumulator_type",
                source: "fn main(){let v=vec![1i32];let _=v.iter().fold(0i32,|a,x| a > *x);}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

/// `Iterator::map` source adapter: a closure-bearing `Map<I, F>` realized by
/// the parent body, consumed by `fold` and by direct `next()`.
#[test]
fn iterator_map_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-map",
        &[
            Case {
                name: "map_then_fold",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2, 3];
    let s = v.iter().map(|x: &i32| *x * 2).fold(0i32, |a: i32, b: i32| a + b);
    println!("{}", s);
}
"#,
                expect: Expect::Run("12\n"),
            },
            Case {
                name: "map_changes_item_type_with_capture",
                source: r#"
fn main() {
    let v: Vec<u8> = vec![3, 4, 5];
    let k = 10u64;
    let m = v.iter().map(|x: &u8| *x as u64 * k);
    println!("{}", m.fold(0u64, |a: u64, b: u64| a + b));
}
"#,
                expect: Expect::Run("120\n"),
            },
            Case {
                name: "map_result_mismatch_rejected",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2, 3];
    let s: bool = v.iter().map(|x: &i32| *x * 2).fold(0i32, |a: i32, b: i32| a + b);
}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

/// `Iterator::filter` source adapter: `Filter<I, P>` with the
/// `P: FnMut(&I::Item) -> bool` projection, consumed by `next()`. The parent
/// may re-check and re-mint an equal closure shape seal.
#[test]
fn iterator_filter_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-filter",
        &[
            Case {
                name: "filter_owned_items_next",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2, 3, 4];
    let mut m = v.iter().copied().filter(|x: &i32| *x % 2 == 0);
    println!("{:?}", m.next());
}
"#,
                expect: Expect::Run("Some(2)\n"),
            },
            Case {
                name: "map_then_next",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![5, 6];
    let mut m = v.iter().map(|x: &i32| *x * 2);
    println!("{:?}", m.next());
}
"#,
                expect: Expect::Run("Some(10)\n"),
            },
            Case {
                name: "filter_predicate_not_bool",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2];
    let mut m = v.iter().copied().filter(|x: &i32| *x);
    println!("{:?}", m.next());
}
"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "filter_predicate_takes_item_by_value",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2];
    let mut m = v.iter().copied().filter(|x: i32| x > 1);
    println!("{:?}", m.next());
}
"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}
