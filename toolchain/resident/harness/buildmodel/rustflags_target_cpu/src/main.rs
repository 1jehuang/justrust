#[inline(never)]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn main() {
    let n = std::env::args().count();
    let a: Vec<f32> = (0..64).map(|i| (i + n) as f32).collect();
    println!("avx2 {} fma {} sse2 {} crt-static {}", cfg!(target_feature = "avx2"), cfg!(target_feature = "fma"),
        cfg!(target_feature = "sse2"), cfg!(target_feature = "crt-static"));
    println!("{}", dot(&a, &a));
    #[cfg(target_arch = "x86_64")]
    println!("detected avx2 at run time: {}", std::is_x86_feature_detected!("avx2") || !cfg!(target_feature = "avx2"));
}
