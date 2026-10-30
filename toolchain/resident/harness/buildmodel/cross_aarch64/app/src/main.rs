#[cfg(all(target_arch = "x86_64", target_env = "gnu"))]
use on_gnu as only;
#[cfg(target_env = "musl")]
use on_musl as only;
#[cfg(target_arch = "aarch64")]
use on_arm as only;
fn main() {
    println!("arch {} os {} env {:?} family {} pointer {} endian {}", std::env::consts::ARCH, std::env::consts::OS,
        option_env!("CARGO_CFG_TARGET_ENV"), std::env::consts::FAMILY, std::mem::size_of::<usize>(), cfg!(target_endian = "little"));
    println!("cfg: linux {} musl {} gnu {} aarch64 {} x86_64 {} unix {}", cfg!(target_os = "linux"), cfg!(target_env = "musl"),
        cfg!(target_env = "gnu"), cfg!(target_arch = "aarch64"), cfg!(target_arch = "x86_64"), cfg!(unix));
    println!("{}", only::name());
    let words: Vec<String> = std::env::args().skip(1).map(|it| it.to_uppercase()).collect();
    println!("{words:?} {}", u64::MAX as f64 / 3.0);
}
