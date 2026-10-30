// Found by fuzz.py: seed L2-431, case c24, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 63 builds. Again: fuzz.py check differs-checksum-L2-431-c24.rs
// rustc:
// c24 a06824e87592de43 ok
// ra-jit:
// c24 3ef78de5c71c8ace ok
mod c24 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub enum E1 {
        V0,
        V1,
    }
    impl Mix for E1 {
        fn mix(&self) {
            match self {
                E1::V0 => { feed(10); }
                E1::V1 => { feed(11); }
            }
        }
    }
    impl Drop for E1 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    pub fn f76(a77: &[E1], a78: &[E1], a79: &mut i8, a80: &[String], a81: &[bool], a82: &mut E1, a83: &bool, a84: &mut E1) -> f64 {
        let [E1::V0, p96 @ .., E1::V0] = (Some([E1::V1, E1::V1, E1::V1, E1::V1, E1::V1, E1::V0, E1::V0, E1::V1]).unwrap_or_else(|| -> [E1; 8] { [E1::V0, E1::V1, E1::V0, E1::V1, E1::V1, E1::V1, E1::V1, E1::V1] })) else { return (if ((*a83)) { 65535.0f64 } else { 1.5f64 }); };
        ((if ((*a83)) { 650.2138f64 } else { (-1.0f64) }) % ((-136416483592556146649886500685404160609i128) as f64))
    }
    pub fn run() {
        let mut n221 = 0u32;
        let v223: f64 = 'l222: loop {
            n221 += 1;
            if n221 > 1 { break ({ let mut m224: i8 = 126i8; let mut m225: E1 = E1::V0; let mut m226: E1 = E1::V1; let out = f76(&[E1::V0, E1::V0, E1::V0], &[E1::V0, E1::V1, E1::V1], &mut m224, &[String::from("Abc Def"), String::from("a,b,,c"), String::from("-7"), String::from("line\n"), String::from("ff")], &[false, false], &mut m225, &false, &mut m226); m224.mix(); m225.mix(); m226.mix(); out }); }
        };
    }
}
