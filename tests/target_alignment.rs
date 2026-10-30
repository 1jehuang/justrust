//! Genuine target alignment is distinct from the compiler's private storage stride.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;

fn stock(source: &str, tag: &str) -> (std::path::PathBuf, std::process::Output) {
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-target-alignment-{}-{tag}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.rs"), source).unwrap();
    let output = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(root.join("main.rs"))
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    (root, output)
}

#[test]
fn target_alignment_scalars_arrays_structs_and_generic_instances() {
    let source = r#"
struct Narrow { first: u8, second: bool }
struct Wide { small: u8, wide: u128 }
struct Nested { inner: Wide, tail: [u16; 3] }
fn alignment<T>() -> usize { std::mem::align_of::<T>() }
fn main() {
    println!("{} {} {} {} {}", alignment::<()>(), alignment::<bool>(), alignment::<char>(), alignment::<f32>(), alignment::<f64>());
    println!("{} {} {} {} {}", alignment::<u8>(), alignment::<u16>(), alignment::<u32>(), alignment::<u64>(), alignment::<u128>());
    println!("{} {} {} {} {}", alignment::<i8>(), alignment::<i16>(), alignment::<i32>(), alignment::<i64>(), alignment::<i128>());
    println!("{} {} {} {}", alignment::<usize>(), alignment::<isize>(), alignment::<*const u8>(), alignment::<*mut u128>());
    println!("{} {} {} {}", alignment::<[u8; 0]>(), alignment::<[u16; 7]>(), alignment::<[u128; 3]>(), alignment::<[[u32; 2]; 4]>());
    println!("{} {} {}", alignment::<Narrow>(), alignment::<Wide>(), alignment::<Nested>());
}
"#;
    let (root, compiled) = stock(source, "native");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
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

#[test]
fn target_alignment_private_operation_rejects_foreign_source_owners() {
    for source in [
        "fn main(){let value=__justrust_align_of!(u8);}",
        "mod mem { pub fn align_of<T>()->usize { __justrust_align_of!(T) } } fn main(){let value=mem::align_of::<u128>();}",
        "mod __justrust_std {pub mod mem {pub fn align_of<T>()->usize {__justrust_align_of!(T)}}} fn main(){let value=__justrust_std::mem::align_of::<u8>();}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile(source.into(), workers, check)
                    .err()
                    .expect("invalid or unsupported alignment must be rejected")
                    .to_string();
                assert!(
                    error.contains("private target alignment")
                        || error.contains("canonical source")
                        || error.contains("duplicate native prelude module"),
                    "{error}"
                );
            }
        }
    }
}

#[test]
fn target_alignment_unknown_layouts_fail_explicitly_instead_of_using_private_storage() {
    for (tag, source) in [
        (
            "enum",
            "enum E {A, B(u128)} fn main(){println!(\"{}\",std::mem::align_of::<E>());}",
        ),
        (
            "unsized-pointer",
            "fn main(){println!(\"{}\",std::mem::align_of::<*const str>());}",
        ),
        (
            "closure",
            "fn alignment<T>(value:T)->usize{std::mem::align_of::<T>()} fn main(){let closure=||1i32; println!(\"{}\",alignment(closure));}",
        ),
        (
            "overaligned",
            "#[repr(align(64))] struct Aligned {field:u8} fn main(){println!(\"{}\",std::mem::align_of::<Aligned>());}",
        ),
    ] {
        let (root, compiled) = stock(source, tag);
        assert!(
            compiled.status.success(),
            "{tag}: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile(source.into(), workers, check)
                    .err()
                    .expect("invalid or unsupported alignment must be rejected")
                    .to_string();
                if tag == "overaligned" {
                    // The attribute parser rejects this spelling before a layout query
                    // can be issued. This is an ingress boundary, not layout support.
                    assert!(
                        error.contains("expected attribute identifier"),
                        "{tag}: {error}"
                    );
                } else {
                    assert!(
                        error.to_lowercase().contains("unsupported"),
                        "{tag}: workers={workers}, check={check}: {error}"
                    );
                }
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}

fn physical_matches_stock(source: &str, tag: &str) {
    let (root, compiled) = stock(source, tag);
    assert!(
        compiled.status.success(),
        "{tag}: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let expected = Command::new(root.join("stock")).output().unwrap();
    assert!(expected.status.success(), "{tag}: {expected:?}");
    for workers in [1, 8] {
        let checked = compiler::compile(source.into(), workers, true).unwrap();
        assert!(
            checked.executable.is_none(),
            "CHECK must not emit an executable"
        );
        let built = compiler::compile(source.into(), workers, false).unwrap();
        let binary = root.join(format!("native-{workers}"));
        fs::write(&binary, built.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(binary).output().unwrap();
        assert!(
            actual.status.success(),
            "{tag}: workers={workers}: {actual:?}"
        );
        assert_eq!(actual.stdout, expected.stdout, "{tag}: workers={workers}");
        assert_eq!(actual.stderr, expected.stderr, "{tag}: workers={workers}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn physical_scalar_pointer_and_integer_address_strides_match_stock() {
    physical_matches_stock(
        r#"
fn main(){
    let mut bytes=[3u8,7u8,11u8];let p=&mut bytes[0]as *mut u8;
    unsafe{assert_eq!(*p.add(2usize),11u8);*p.add(1usize)=9u8;assert_eq!(*p.offset(1isize),9u8);assert_eq!(p.add(2usize).offset_from(p as *const u8),2isize);}
    assert_eq!(bytes[1],9u8);assert!(p.wrapping_add(2usize).wrapping_sub(2usize)==p);
    let words=[13u64,29u64];let q=&words[0]as *const u64;
    unsafe{assert_eq!(*q.add(1usize),29u64);assert_eq!(q.add(1usize).offset_from(q),1isize);}
    let narrow=[257u16,513u16,769u16];let r=&narrow[0]as *const u16;
    unsafe{assert_eq!(*r.add(1usize),513u16);assert_eq!(r.add(2usize).offset_from(r),2isize);}
    let raw=r as *const u8;unsafe{println!("{} {} {} {}",*raw,*raw.add(1usize),*raw.add(2usize),*raw.add(3usize));}
    let next=(r as usize+std::mem::size_of::<u16>())as *const u16;unsafe{assert_eq!(*next,513u16);}
    let signed=[-7i32,-900i32];let sp=&signed[0]as *const i32;unsafe{assert_eq!(*sp.add(1usize),-900i32);assert_eq!(sp.add(1usize).offset_from(sp),1isize);}
    let flags=[false,true];let fp=&flags[0]as *const bool;unsafe{assert_eq!(*fp.add(1usize),true);}assert_eq!(fp.wrapping_add(1usize)as usize-fp as usize,1usize);
    let units=[(),()];let zp=&units[0]as *const ();assert!(zp.wrapping_add(1usize)==zp);
    let rows=[*b"abc",*b"xyz"];let ap=&rows[0]as *const[u8;3];unsafe{let second=*ap.add(1usize);assert_eq!(second[2],b'z');assert_eq!(ap.add(1usize)as usize-ap as usize,3usize);}
    println!("pointer strides ok");
}"#,
        "physical_scalar_pointer_and_integer_address_strides_match_stock",
    );
}

#[test]
fn physical_recursive_array_copies_and_scalar_memory_match_stock() {
    physical_matches_stock(
        r#"
fn relay<T:Copy>(value:T)->T{value}
fn next<T>(pointer:*const T)->*const T{(pointer as usize+std::mem::size_of::<T>())as *const T}
fn change(values:&mut[u16]){values[1usize]=60000u16;let at=2usize;values[at]+=7u16;}
struct Holder{before:u64,words:[u16;3],after:u64}
fn main(){
    let mut holder=Holder{before:99u64,words:[257u16,513u16,769u16],after:101u64};
    change(&mut holder.words);assert_eq!(holder.words[0],257u16);assert_eq!(holder.words[1],60000u16);assert_eq!(holder.words[2],776u16);assert_eq!(holder.before,99u64);assert_eq!(holder.after,101u64);
    let original=holder.words;holder.words[1]=17u16;assert_eq!(holder.words[1],17u16);assert_eq!(original[1],60000u16);
    let p=&original[0]as *const u16;unsafe{assert_eq!(*next(p),60000u16);}assert_eq!((&original[2]as *const u16 as usize)-(p as usize),4usize);
    let repeats=relay([513u16;17]);assert_eq!(repeats[0],513u16);assert_eq!(repeats[16],513u16);
    let nested=relay([[257u16,513u16,769u16];3]);let row=nested[2];assert_eq!(row[2],769u16);let a=&nested[0]as *const[u16;3];unsafe{let row=*next(a);assert_eq!(row[1],513u16);}assert_eq!((&nested[2]as *const[u16;3]as usize)-(a as usize),12usize);
    let short=relay([[1u8,2u8,3u8];3]);let a=&short[0]as *const[u8;3];let bytes=a as *const u8;unsafe{assert_eq!(*bytes.add(3usize),1u8);let row=*next(a);assert_eq!(row[2],3u8);}
    let mut signed=relay([-3i8;9]);signed[8]=-127i8;let index=8usize;assert_eq!(signed[index],-127i8);assert_eq!(signed[7],-3i8);
    let mut half=[-300i16;5];half[3]=-17000i16;let h=&half[0]as *const i16;unsafe{assert_eq!(*h.add(3usize),-17000i16);}assert_eq!(half[4],-300i16);
    let mut ints=[-70000i32;3];ints[1]=-900000i32;let i=&ints[0]as *const i32;unsafe{assert_eq!(*next(i),-900000i32);}assert_eq!(ints[2],-70000i32);
    let chars=relay(['é','界','🦀']);let c=&chars[0]as *const char;unsafe{assert_eq!(*next(c),'界');}assert_eq!((&chars[2]as *const char as usize)-(c as usize),8usize);
    let mut floats=[1.5f32;5];floats[3]=2.25f32;let f=&floats[0]as *const f32;unsafe{assert_eq!(*f.add(3usize),2.25f32);}assert_eq!(floats[4],1.5f32);
    let doubles=[1.25f64,2.5f64];let d=&doubles[0]as *const f64;unsafe{assert_eq!(*next(d),2.5f64);}
    let wide=[170141183460469231731687303715884105729u128;2];let w=&wide[0]as *const u128;unsafe{assert_eq!(*next(w),wide[1]);}
    let empty=[[();3];2];let z=&empty[0]as *const[();3];assert!(z.wrapping_add(1usize)==z);
    println!("scalar-memory");
}"#,
        "physical_recursive_array_copies_and_scalar_memory_match_stock",
    );
}

#[test]
fn physical_signed_alias_stores_normalize_carriers_match_stock() {
    physical_matches_stock(
        r#"fn set8(x:&mut i8,value:i8){*x=value;}fn read8(x:&i8)->i8{*x}
fn set16(x:&mut i16,value:i16){*x=value;}fn read16(x:&i16)->i16{*x}
fn set32(x:&mut i32,value:i32){*x=value;}fn read32(x:&i32)->i32{*x}
#[derive(Clone,Copy)]struct Leaf{a:i8,b:i16,c:i32}
fn main(){
 let mut a=-1i8;let p=&mut a as *mut i8;unsafe{*p=2i8;}assert_eq!(a,2i8);assert_eq!(read8(&a),2i8);let p=&mut a as *mut i8;unsafe{*p=-3i8;}assert_eq!(a,-3i8);set8(&mut a,5i8);assert_eq!(a,5i8);set8(&mut a,-7i8);assert_eq!(read8(&a),-7i8);
 let mut b=-1i16;let p=&mut b as *mut i16;unsafe{*p=2i16;}assert_eq!(b,2i16);assert_eq!(read16(&b),2i16);let p=&mut b as *mut i16;unsafe{*p=-300i16;}assert_eq!(b,-300i16);set16(&mut b,513i16);assert_eq!(b,513i16);set16(&mut b,-17000i16);assert_eq!(read16(&b),-17000i16);
 let mut c=-1i32;let p=&mut c as *mut i32;unsafe{*p=2i32;}assert_eq!(c,2i32);assert_eq!(read32(&c),2i32);let p=&mut c as *mut i32;unsafe{*p=-90000i32;}assert_eq!(c,-90000i32);set32(&mut c,70000i32);assert_eq!(c,70000i32);set32(&mut c,-190000i32);assert_eq!(read32(&c),-190000i32);
 let mut leaf=Leaf{a:-1i8,b:-1i16,c:-1i32};let pa=&mut leaf.a as *mut i8;let pb=&mut leaf.b as *mut i16;let pc=&mut leaf.c as *mut i32;unsafe{*pa=2i8;*pb=257i16;*pc=70000i32;}let copied=leaf;assert_eq!(copied.a,2i8);assert_eq!(copied.b,257i16);assert_eq!(copied.c,70000i32);
 set8(&mut leaf.a,-7i8);set16(&mut leaf.b,-17000i16);set32(&mut leaf.c,-190000i32);assert_eq!(read8(&leaf.a),-7i8);assert_eq!(read16(&leaf.b),-17000i16);assert_eq!(read32(&leaf.c),-190000i32);assert_eq!(copied.a,2i8);assert_eq!(copied.b,257i16);assert_eq!(copied.c,70000i32);
 println!("{} {} {}",a,b,c);println!("{} {} {}",leaf.a,leaf.b,leaf.c);
}"#,
        "physical_signed_alias_stores_normalize_carriers_match_stock",
    );
}

#[test]
fn physical_u16_byte_reinterpretation_matches_stock() {
    physical_matches_stock(
        r#"fn main(){let a=[257u16,513u16];let p=&a[0]as *const u16;let bytes=p as *const u8;unsafe{println!("{} {} {} {}",*bytes,*bytes.add(1usize),*bytes.add(2usize),*bytes.add(3usize));}}"#,
        "physical_u16_byte_reinterpretation_matches_stock",
    );
}
