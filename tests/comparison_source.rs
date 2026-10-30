//! Original source identities survive comparison autoref, including nested calls.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn ordering_total_order_matches_stock_for_every_variant_pair() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "ordering-total-order",
        &[Case {
            name: "all_pairs_and_generic_defaults",
            source: r#"use std::cmp::Ordering;
fn pair(a:Ordering,b:Ordering){
    let c=a.cmp(&b);
    println!("{} {} {} {}",c.is_lt(),c.is_eq(),c.is_gt(),a.partial_cmp(&b).unwrap()==c);
}
fn choose<T:Ord>(a:T,b:T)->T{a.min(b)}
fn main(){
    pair(Ordering::Less,Ordering::Less);
    pair(Ordering::Less,Ordering::Equal);
    pair(Ordering::Less,Ordering::Greater);
    pair(Ordering::Equal,Ordering::Less);
    pair(Ordering::Equal,Ordering::Equal);
    pair(Ordering::Equal,Ordering::Greater);
    pair(Ordering::Greater,Ordering::Less);
    pair(Ordering::Greater,Ordering::Equal);
    pair(Ordering::Greater,Ordering::Greater);
    let lo=Ordering::Less;let hi=Ordering::Greater;
    println!("{} {} {} {}",choose(lo,hi).is_lt(),lo.max(hi).is_gt(),Ordering::Equal.clamp(lo,hi).is_eq(),choose(&lo,&hi).is_lt());
}"#,
            expect: Expect::Run(
                "false true false true\ntrue false false true\ntrue false false true\nfalse false true true\nfalse true false true\ntrue false false true\nfalse false true true\nfalse false true true\nfalse true false true\ntrue true true true\n",
            ),
        }],
    );
}

#[test]
fn reference_ordering_forwards_exact_pointee_traits() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "reference-ordering",
        &[
            Case {
                name: "unsized_str_and_nested_scalar",
                source: r#"fn compare<T:Ord>(left:T,right:T)->bool{left.cmp(&right).is_lt()}fn partial<T:PartialOrd>(left:T,right:T)->bool{left.partial_cmp(&right).unwrap().is_lt()}fn main(){let a=1i32;let b=2i32;let ra=&a;let rb=&b;println!("{} {} {} {}",compare("a","hé"),compare(ra,rb),compare(&ra,&rb),partial("a","b"));}"#,
                expect: Expect::Run("true true true true\n"),
            },
            Case {
                name: "nominal_option_ordering_result",
                source: r#"fn compare<T:PartialOrd>(left:T,right:T)->Option<std::cmp::Ordering>{left.partial_cmp(&right)}fn main(){let a=1i32;let b=2i32;println!("{} {}",compare(&a,&b).unwrap().is_lt(),compare("a","b").unwrap().is_lt());}"#,
                expect: Expect::Run("true true\n"),
            },
            Case {
                name: "partial_order_keeps_nan_unordered",
                source: r#"fn partial<T:PartialOrd>(left:T,right:T)->bool{left.partial_cmp(&right).is_none()}fn main(){let nan=0.0f64/0.0f64;let one=1.0f64;println!("{}",partial(&nan,&one));}"#,
                expect: Expect::Run("true\n"),
            },
            Case {
                name: "unused_reference_without_pointee_ord",
                source: "struct N;fn require<T:Ord>(_:T){}fn unused(){let value=N;require(&value);}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "named_input_regions_valid_control",
                source: r#"trait Compare{fn same<'a,'b>(&self,left:&'a i32,right:&'b i32)->bool;}struct N;impl Compare for N{fn same<'a,'b>(&self,left:&'a i32,right:&'b i32)->bool{*left==*right}}fn main(){let a=1;let b=1;println!("{}",N.same(&a,&b));}"#,
                expect: Expect::Run("true\n"),
            },
            Case {
                name: "implementation_strengthens_invariant_input_equality",
                source: r#"trait Compare{fn same<'a,'b>(&self,left:&mut &'a i32,right:&mut &'b i32)->bool;}struct N;impl Compare for N{fn same<'a>(&self,left:&mut &'a i32,right:&mut &'a i32)->bool{false}}fn main(){}"#,
                expect: Expect::Invalid("error[E"),
            },
            Case {
                name: "implementation_strengthens_input_outlives",
                source: r#"trait Compare{fn same<'a,'b>(&self,left:&'a i32,right:&'b i32)->bool;}struct N;impl Compare for N{fn same<'a:'b,'b>(&self,left:&'a i32,right:&'b i32)->bool{false}}fn main(){}"#,
                expect: Expect::Invalid("error[E"),
            },
            Case {
                name: "unused_float_reference_has_no_total_order",
                source: "fn require<T:Ord>(_:T){}fn unused(){let value=1.0f64;require(&value);}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn shared_local_comparison_inputs_keep_mutability_and_escape_checks() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "shared-local-inputs",
        &[
            Case {
                name: "two_local_shared_reference_depths_without_let_result",
                source: r#"fn compare<T:Ord>(left:T,right:T)->bool{left.cmp(&right).is_lt()}fn main(){let a=1i32;let b=2i32;let ra=&a;let rb=&b;println!("{} {}",compare(ra,rb),compare(&ra,&rb));}"#,
                expect: Expect::Run("true true\n"),
            },
            Case {
                name: "root_shadowing_preserves_both_reference_occurrences",
                source: r#"fn compare<T:Ord>(a:T,b:T)->bool{a.cmp(&b).is_lt()}fn main(){let a=1i32;let r=&a;let a=3i32;let s=&a;println!("{} {}",compare(r,s),compare(&r,&s));}"#,
                expect: Expect::Run("true true\n"),
            },
            Case {
                name: "parameter_shadowing_keeps_original_occurrence",
                source: r#"fn compare<T:Ord>(a:T,b:T)->bool{a.cmp(&b).is_lt()}fn probe(a:i32)->bool{let r=&a;let a=3i32;let s=&a;compare(&r,&s)}fn main(){println!("{}",probe(1i32));}"#,
                expect: Expect::Run("true\n"),
            },
            Case {
                name: "nongeneric_and_explicit_multislot_calls_keep_earlier_routes",
                source: r#"fn plain(a:&i32,b:&i32)->bool{*a<*b}fn generic<T,U>(a:T,b:U)->bool{true}fn mixed<T>(a:T,b:i32)->bool{true}fn main(){let a=1i32;let b=2i32;let r=&a;let s=&b;println!("{} {} {} {}",plain(r,s),generic::<&i32,&i32>(r,s),generic(a,b),mixed::<&i32>(r,b));}"#,
                expect: Expect::Run("true true true true\n"),
            },
            Case {
                name: "shared_local_cannot_gain_mutable_authority",
                source: "fn needs_mut(_: &mut i32){}fn main(){let a=1i32;let r=&a;needs_mut(r);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "primitive_result_call_does_not_make_local_reference_escape",
                source: "fn probe<T>(left:T,right:T)->bool{true}fn bad<'a>()->&'a i32{let x=1i32;let y=2i32;probe(&x,&y);&x}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
        ],
    );
}

#[test]
fn heterogeneous_string_equality_preserves_reference_contracts() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "heterogeneous-equality",
        &[
            Case {
                name: "string_and_str",
                source: r#"fn main(){let s=String::from("héllo");let t="héllo";println!("{} {} {} {}",s==t,t==s,s!="bye","bye"!=s);println!("{}",s);}"#,
                expect: Expect::Run("true true true true\nhéllo\n"),
            },
            Case {
                name: "custom_borrowed_rhs",
                source: r#"struct Item{value:i32}impl PartialEq<&i32> for Item{fn eq(&self,other:&&i32)->bool{self.value==**other}}fn main(){let item=Item{value:7};let n=7;println!("{} {}",item==&n,item!=&8);}"#,
                expect: Expect::Run("true true\n"),
            },
            Case {
                name: "invalid_rhs_unused",
                source: r#"fn bad(){let s=String::from("x");let same=s==3i32;}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn comparison_autoref_preserves_original_nested_calls() {
    let source = r#"
struct Item { value: i32 }
impl PartialEq for Item {
    fn eq(&self, other: &Item) -> bool { self.value == other.value }
}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Item) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}
impl Item {
    fn same(&self) -> Item { println!("same {}", self.value); Item { value: self.value } }
}
fn make(value: i32) -> Item { println!("make {}", value); Item { value } }
fn main() {
    let item = Item { value: 7 };
    println!("{}", item == item.same());
    println!("{}", make(1) != make(2));
    println!("{}", make(1) < make(2));
    println!("{}", make(1) <= make(2));
    println!("{}", make(2) > make(1));
    println!("{}", make(2) >= make(1));
    println!("{}", item.value);
}
"#;
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-comparison-source-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.rs"), source).unwrap();
    let stock = Command::new("rustc")
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(root.join("main.rs"))
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let expected = Command::new(root.join("stock")).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        compiler::compile(source.into(), workers, true).unwrap();
        let built = compiler::compile(source.into(), workers, false).unwrap();
        let binary = root.join(format!("native-{workers}"));
        fs::write(&binary, built.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(binary).output().unwrap();
        assert!(actual.status.success(), "{actual:?}");
        assert_eq!(actual.stdout, expected.stdout, "workers={workers}");
    }
    fs::remove_dir_all(root).unwrap();
}
