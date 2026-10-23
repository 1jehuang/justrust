//! Original source identities survive comparison autoref, including nested calls.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

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
