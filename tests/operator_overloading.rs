//! Overloaded operators lower to std::ops trait calls.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn run(source: &str, expected: &str) {
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
        let path = std::env::temp_dir().join(format!(
            "jr-operators-{}-{workers}-{}",
            std::process::id(),
            source.len()
        ));
        fs::write(&path, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&path).output().unwrap();
        let _ = fs::remove_file(&path);
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "{source}"
        );
    }
}

const VEC2: &str = "use std::ops::{Add, Mul, Neg, Sub, AddAssign};
struct V { x: i32, y: i32 }
impl Clone for V { fn clone(&self) -> V { *self } }
impl Copy for V {}
impl Add for V { type Output = V; fn add(self, o: V) -> V { V { x: self.x + o.x, y: self.y + o.y } } }
impl Sub for V { type Output = V; fn sub(self, o: V) -> V { V { x: self.x - o.x, y: self.y - o.y } } }
impl Mul<i32> for V { type Output = V; fn mul(self, k: i32) -> V { V { x: self.x * k, y: self.y * k } } }
impl Neg for V { type Output = V; fn neg(self) -> V { V { x: -self.x, y: -self.y } } }
impl AddAssign for V { fn add_assign(&mut self, o: V) { self.x += o.x; self.y += o.y; } }
";

#[test]
fn binary_unary_and_compound_operators() {
    run(
        &format!(
            "{VEC2}fn main() {{ let a = V {{ x: 1, y: 2 }}; let b = V {{ x: 3, y: 4 }}; let c = a + b; let d = c - a; let e = -(d * 3); let mut f = e; f += a; println!(\"{{}} {{}} {{}} {{}}\", c.x, d.y, e.x, f.y); }}"
        ),
        "4 4 -9 -10\n",
    );
}

#[test]
fn missing_operator_impl_is_rejected() {
    let source = "struct V { x: i32 } fn main() { let a = V { x: 1 }; let b = V { x: 2 }; let c = a + b; println!(\"{}\", c.x); }";
    let error = compiler::compile(source.into(), 1, true)
        .err()
        .expect("accepted");
    assert!(error.to_string().contains("E0369"), "{error}");
}

#[test]
fn self_in_impl_trait_arguments_names_the_impl_self_type() {
    // Adler2's U32X4 pattern: `impl AddAssign<Self> for U32X4`.
    run(
        "use std::ops::{AddAssign, MulAssign, RemAssign};
        #[derive(Copy, Clone)] struct U([u32; 4]);
        impl AddAssign<Self> for U { fn add_assign(&mut self, other: Self) { self.0[0] += other.0[0]; self.0[3] += other.0[3]; } }
        impl MulAssign<u32> for U { fn mul_assign(&mut self, rhs: u32) { self.0[0] *= rhs; } }
        impl RemAssign<u32> for U { fn rem_assign(&mut self, q: u32) { self.0[3] %= q; } }
        struct W<T>(T);
        trait Pair<A> { fn pair(&self, other: &A) -> u32; }
        impl Pair<(Self, u8)> for W<u32> { fn pair(&self, other: &(Self, u8)) -> u32 { self.0 + (other.0).0 + other.1 as u32 } }
        fn main() { let mut a = U([1, 2, 3, 9]); a += U([4, 0, 0, 8]); a *= 3; a %= 5; let w = W(10u32); println!(\"{} {} {}\", a.0[0], a.0[3], w.pair(&(W(5u32), 2u8))); }",
        "15 2 17\n",
    );
}

#[test]
fn self_in_impl_trait_arguments_is_not_another_type() {
    for source in [
        "use std::ops::AddAssign; struct A(u32); struct B(u32); impl AddAssign<Self> for A { fn add_assign(&mut self, o: Self) { self.0 += o.0; } } fn main() { let mut a = A(1); a += B(2); }",
        "trait T<X> { fn f(x: X); } struct A; impl T<Self> for A { fn f(x: u8) {} } fn main() {}",
        "trait T<X> {} struct A; fn g() -> Self { loop {} } impl T<Self> for A {} fn main() {}",
    ] {
        for check in [true, false] {
            assert!(
                compiler::compile(source.into(), 1, check).is_err(),
                "accepted: {source}"
            );
        }
    }
}
