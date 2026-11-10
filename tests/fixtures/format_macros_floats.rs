use std::fmt::Write;
fn main() {
    let zero = 0.0f64;
    let nan = zero / zero;
    let inf = 1.0f64 / zero;
    // Shortest round-trip digits are found per width.
    println!("{} {:?} {} {:?}", 1.1f32, 1.1f32, 0.1f32 + 0.2f32, 0.1f64 + 0.2f64);
    println!("{} {:?}", 3.4028235e38f32, 1e-45f32);
    println!("{} {:?}", 1.5e300f64, 1.5e300f64);
    println!("{} {:?}", 5e-324f64, 5e-324f64);
    println!("{} {:?}", 1.7976931348623157e308f64, 2.2250738585072014e-308f64);
    // Debug switches to exponent form below 1e-4 and from 1e16.
    println!("{:?} {:?} {:?} {:?}", 1e-7f64, 1e-5f64, 1e-4f64, 0.00012f32);
    println!("{:?} {:?} {:?} {:?}", 1e15f64, 9999999999999998.0f64, 1e16f64, 1e16f32);
    println!("{:?} {:?} {:?} {}", 0.0f64, -0.0f64, 1.0f32, -0.0f32);
    println!("{:?} {:?}", vec![1.5f32, 1e-10, 3.0e20], (0.1f64, 2.0f32));
    // Fixed precision rounds the exact binary value half-to-even.
    println!("{:.0} {:.0} {:.0} {:.0} {:.1} {:.2}", 0.5, 1.5, 2.5, 0.51, 2.55, 1.005);
    println!("{:.3} {:.3} {:.1} {:.20}", 0.0004f64, 0.0005f64, 0.96f32, 0.1f64);
    println!("{:.2} {:.0} {:.3}", 1e20f64, 1.5e300f64, 9.9995f32);
    println!("{:.30} {:.5}", 5e-324f64, 1.0e-10f64);
    // Exponent forms for floats and integers.
    println!("{:e} {:E} {:e} {:e} {:e}", 1234.5f64, 0.00012f32, 0.0f64, -0.0f64, 1e100f64);
    println!("{:.2e} {:.0e} {:.0e} {:.3e} {:.4E}", 123456.0f64, 2.5f64, 3.5f64, 0.0f64, 9.99996f32);
    println!("{:e} {:E} {:.2e} {:e} {:.1e} {:e} {:e}", 1234, 150u8, 123456, 0, 125i64, -1200i32, u128::MAX);
    // Width, fill, sign and zero flags.
    println!("[{:10.3e}] [{:<10e}] [{:+e}] [{:010.2E}]", 1234.5f64, 0.00012f32, 1.0f64, -1234.5f64);
    println!("[{:08.2}] [{:+}] [{:^9.1}] [{:*<8}] [{:5?}] [{:.2?}]", -1.5f64, 2.0f32, 2.25, 1.5, 1.0, 1e20);
    println!("[{:05}] [{:+}] [{:.1}] [{:8}] [{:<8}] [{:?}] [{:e}]", nan, nan, nan, nan, nan, nan, nan);
    println!("[{:05}] [{:+}] [{:.1}] [{:8}] [{:<8}] [{:?}] [{:+08.2}]", inf, inf, inf, inf, inf, -inf, -inf);
    let mut s = String::new();
    write!(s, "{:e}|{:.2E}|{:>10.3}|{:?}", 1.5f32, 123456.0f64, 2.0f32, &7.5f64).unwrap();
    println!("{s} {}", format!("{:e} {:.1e}", 1200u32, 999u16));
}
