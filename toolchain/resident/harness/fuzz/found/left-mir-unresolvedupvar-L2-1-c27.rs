// Found by fuzz.py: seed L2-1, case c27, edition 2024, backend cranelift.
// left: mir: UnresolvedUpvar
// Reduced in 57 builds. Again: fuzz.py check left-mir-unresolvedupvar-L2-1-c27.rs
// fz::c27::run: unsupported: callee `fold`: callee `fold`: callee `fold`: callee `spec_fold`: callee `try_fold`: callee `try_fold`: callee `{closure}`: callee `{closure}`: callee `{closure}`: callee `{closure}`: frontend: mir: UnresolvedUpvar(StoredPlace { local: Idx::<Local>(5), projection: [Field(FieldIndex(1))] })
mod c27 {
    use super::*;
    static K2: char = '\u{80}';
    pub fn run() {
        'l110: for x109 in (std::iter::repeat((match (('\\', 16i8)) { ('é', -69i8) => K2, _ => '9' })).take(3).zip(({ let mut items = Vec::<(char, i8)>::new(); items.retain(|x108: &(char, i8)| -> bool { true }); items }))) {
            let mut v136: Vec<u128> = ({ let mut items = Ok::<Vec<u128>, i128>(vec![8796093022207u128, 1025u128]).unwrap_or(vec![3u128, 92895926383679905491471429206777737770u128, 161547900399298305174717613792394178042u128]); items.push(70682229848101329384799192001445829923u128.wrapping_shl(1472423475u32)); items });
            'l138: while let Some(x137) = v136.pop() {
                let v142: isize = std::iter::repeat(K2.to_string().to_lowercase()).take(0).filter(|x143: &String| -> bool { (!('b144: { if (false) { break 'b144 true; } false })) }).filter(|x145: &String| -> bool { (match (x109.1) { (_, _) if false => K2, _ => x109.0 }).is_whitespace() }).fold(((match (&{ ('0', 15i8) }) { (p148, 2i8) => f64::NAN, _ => 3.25f64 }) as isize), |acc146: isize, x147: String| -> isize { acc146.wrapping_mul(8619737344383235473isize).wrapping_shl(16383u32.checked_shr(5u32).unwrap_or(12u32)) });
            }
        }
    }
}
