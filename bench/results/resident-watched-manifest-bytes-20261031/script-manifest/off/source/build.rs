use std::{env,fs::{self,OpenOptions},io::Write,path::PathBuf,process::Command,time::UNIX_EPOCH};
fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let secs=fs::metadata("Cargo.toml").unwrap().modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let value=secs%1000;
    let out=PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::create_dir_all(&out).unwrap();
    let source=out.join("manifest_native.c");
    let object=out.join("manifest_native.o");
    let archive=out.join("libmanifestnative.a");
    fs::write(&source,format!("unsigned int native_value(void) {{ return {value}; }}\n")).unwrap();
    let cc=env::var_os("JR_MANIFEST_CC").unwrap();
    let ar=env::var_os("JR_MANIFEST_AR").unwrap();
    assert!(Command::new(cc).arg("-c").arg(&source).arg("-o").arg(&object).status().unwrap().success());
    assert!(Command::new(ar).arg("crs").arg(&archive).arg(&object).status().unwrap().success());
    let mut proof=OpenOptions::new().create(true).append(true).open(env::var_os("JR_MANIFEST_PROOF_LOG").unwrap()).unwrap();
    writeln!(proof,"{} {} {}",secs,value,std::process::id()).unwrap();
    proof.sync_all().unwrap();
    println!("cargo:rustc-link-search=native={}",out.display());
    println!("cargo:rustc-link-lib=static=manifestnative");
}
