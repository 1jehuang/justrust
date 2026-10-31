use std::{env,fs,path::PathBuf,process::Command};
fn main(){
 println!("cargo:rerun-if-env-changed=JR_NATIVE_CHOICE");
 println!("cargo:rerun-if-changed=a.c");println!("cargo:rerun-if-changed=b.c");
 let choice=env::var("JR_NATIVE_CHOICE").unwrap();assert!(choice=="a"||choice=="b");
 let out=PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(&choice);fs::create_dir_all(&out).unwrap();
 let object=out.join("native.o");let archive=out.join("libnativechoice.a");
 assert!(Command::new("cc").args(["-c",&format!("{choice}.c"),"-o"]).arg(&object).status().unwrap().success());
 assert!(Command::new("ar").arg("crs").arg(&archive).arg(&object).status().unwrap().success());
 println!("cargo:rustc-link-search=native={}",out.display());println!("cargo:rustc-link-lib=static=nativechoice");
}
