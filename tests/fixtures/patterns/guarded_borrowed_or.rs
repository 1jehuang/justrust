// Future native acceptance: stock prints "true 2\n". Currently exercised by
// the active SCC-loan boundary test, not counted as native pattern support.
enum E { Pair(bool, bool), Empty }
fn probe(c: &mut i32, value: bool) -> bool { *c += 1; value }
fn main() {
    let mut c = 0;
    let found = match E::Pair(true, false) {
        E::Pair(true, ref x) | E::Pair(ref x, false) if probe(&mut c, *x) => true,
        _ => false,
    };
    println!("{} {}", found, c);
}
