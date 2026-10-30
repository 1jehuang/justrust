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
        "mod mem {pub fn storage_alignment<T>()->usize{__justrust_align_of!(T)}} fn main(){let value=mem::storage_alignment::<u8>();}",
        "mod __justrust_std {pub mod mem {pub fn storage_alignment<T>()->usize{__justrust_align_of!(T)}}} fn main(){let value=__justrust_std::mem::storage_alignment::<u8>();}",
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
            "byte-enum",
            r#"enum E {A,B} fn main(){println!("{}",std::mem::align_of::<E>());}"#,
        ),
        (
            "nested-enum",
            r#"enum E {A,B} struct Wrapped{value:[E;2]} fn main(){println!("{}",std::mem::align_of::<Wrapped>());}"#,
        ),
        (
            "fn-item",
            r#"fn item(){} fn alignment<T>(value:T)->usize{std::mem::align_of::<T>()} fn main(){println!("{}",alignment(item));}"#,
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

#[test]
fn private_storage_alignment_preserves_vec_enum_transport() {
    physical_matches_stock(
        r#"
        enum E { A, B(u16) }
        fn main() {
            let mut values:Vec<E>=Vec::new();
            assert!(values.as_ptr() as usize!=0usize);
            assert!(values.as_mut_ptr() as usize!=0usize);
            assert_eq!(values.as_slice().len(),0usize);
            values.push(E::A); values.push(E::B(513u16));
            let pointer=values.as_ptr();
            let first=unsafe{&*pointer};
            match first {E::A=>{},E::B(_)=>panic!("first")}
            match values.pop(){Some(E::B(value))=>assert_eq!(value,513u16),_=>panic!("payload")}
            match values.pop(){Some(E::A)=>{},_=>panic!("tag")}
            println!("private enum storage");
        }
    "#,
        "private_storage_alignment_vec_enum",
    );
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
        let checked = compiler::compile(source.into(), workers, true)
            .unwrap_or_else(|error| panic!("{tag}: workers={workers} CHECK: {error}"));
        assert!(
            checked.executable.is_none(),
            "CHECK must not emit an executable"
        );
        let built = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|error| panic!("{tag}: workers={workers} BUILD: {error}"));
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
fn physical_wide_frame_addresses_ignore_argument_and_caller_frame_parity() {
    // Keep these sources independent: adding unrelated temporaries can mask
    // the old frame-base parity error. They inspect real addresses, not align_of.
    for (tag, source) in [
        (
            "wide-local-frame",
            r#"fn probe(){let x=1u128;println!("{}",&x as *const u128 as usize%16usize);}fn main(){probe();}"#,
        ),
        (
            "wide-array-frame",
            r#"fn probe(){let x=[1u128;1];println!("{}",&x as *const [u128;1] as usize%16usize);}fn main(){probe();}"#,
        ),
        (
            "wide-caller-parity",
            r#"fn probe()->usize{let x=1u128;&x as *const u128 as usize%16usize}fn caller0()->usize{probe()}fn caller1()->usize{let p=1u64;let r=probe();assert_eq!(p,1u64);r}fn main(){println!("{} {}",caller0(),caller1());}"#,
        ),
        (
            "wide-recursive-parity",
            r#"fn recurse(n:u64){let x=1u128;let a=[2u128;1];println!("{} {}",&x as *const u128 as usize%16usize,&a as *const [u128;1] as usize%16usize);if n>0u64{recurse(n-1u64);}}fn main(){recurse(3u64);}"#,
        ),
        (
            "wide-scalar-args-parity",
            r#"fn odd(a:u64){let x=1u128;println!("{} {}",&x as *const u128 as usize%16usize,a);}fn even(a:u64,b:u64){let x=[1u128;1];println!("{} {} {}",&x as *const [u128;1] as usize%16usize,a,b);}fn main(){odd(7u64);even(7u64,9u64);}"#,
        ),
        (
            "wide-fat-args-parity",
            r#"fn f(s:&[u64]){let x=1u128;println!("{} {}",&x as *const u128 as usize%16usize,s.len());}fn g(a:u64,s:&[u64]){let x=[1u128;1];println!("{} {} {}",&x as *const [u128;1] as usize%16usize,a,s.len());}fn main(){let pad=3u64;let s=[7u64];f(&s);g(9u64,&s);assert_eq!(pad,3u64);}"#,
        ),
    ] {
        physical_matches_stock(source, tag);
    }
}

#[test]
fn physical_wide_frames_arrays_nested_calls_recursion_traits_and_drop_match_stock() {
    physical_matches_stock(
        r#"
fn aligned(value:&u128){assert_eq!(value as *const u128 as usize%16usize,0usize);}
fn array_aligned(value:&[[u128;2];2]){
    assert_eq!(value as *const [[u128;2];2] as usize%16usize,0usize);
    assert_eq!(&value[0] as *const [u128;2] as usize%16usize,0usize);
    assert_eq!(&value[1] as *const [u128;2] as usize%16usize,0usize);
    aligned(&value[0][0]);aligned(&value[0][1]);aligned(&value[1][0]);aligned(&value[1][1]);
}
fn wide(a:u64,b:u64,value:u128)->u128{aligned(&value);assert_eq!(a,7u64);assert_eq!(b,9u64);value+1u128}
fn arrays(value:[[u128;2];2],salt:u64)->[[u128;2];2]{array_aligned(&value);assert_eq!(salt,11u64);value}
fn fat_odd(a:u64,values:&[u64])->usize{let x=19u128;aligned(&x);assert_eq!(a,13u64);assert_eq!(values[1],29u64);values.len()}
fn fat_even(values:&[u64],tail:&[u64])->usize{let x=23u128;aligned(&x);assert_eq!(values[0],17u64);assert_eq!(tail[0],29u64);values.len()+tail.len()}
struct Guard{id:u64}
impl Drop for Guard{fn drop(&mut self){let local=31u128;aligned(&local);println!("drop {}",self.id);}}
fn recursive(depth:u64,value:u128)->u128{
    let guard=Guard{id:depth};let sentinel=depth as u128+101u128;aligned(&value);aligned(&sentinel);
    if depth==0u64{return value;}
    let result=recursive(depth-1u64,value+1u128);aligned(&result);aligned(&sentinel);result
}
trait Probe{fn probe(&self,value:u64)->u128;}
struct Marker{tag:u64}
impl Probe for Marker{fn probe(&self,value:u64)->u128{let local=value as u128+41u128;aligned(&local);assert_eq!(self.tag,43u64);wide(7u64,9u64,local)}}
fn dispatch(object:&dyn Probe,value:u64)->u128{let local=47u128;aligned(&local);object.probe(value)}
fn note(order:&mut u64,digit:u64)->u64{*order=*order*10u64+digit;digit}
fn ordered(a:u64,b:usize,c:u64)->usize{let local=53u128;aligned(&local);assert_eq!(a,1u64);assert_eq!(c,2u64);b}
fn main(){
    let narrow=3u64;let wide_local=18446744073709551617u128;aligned(&wide_local);assert_eq!(narrow,3u64);
    let signed=-18446744073709551617i128;assert_eq!(&signed as *const i128 as usize%16usize,0usize);
    let array=arrays([[wide_local,2u128],[3u128,4u128]],11u64);array_aligned(&array);assert_eq!(array[0][0],wide_local);
    let zero:[u128;0]=[];assert_eq!(&zero as *const [u128;0] as usize%16usize,0usize);
    let row_zero:[[u128;0];2]=[[],[]];assert_eq!(&row_zero as *const [[u128;0];2] as usize%16usize,0usize);
    let result=wide(7u64,9u64,wide(7u64,9u64,wide_local));aligned(&result);assert_eq!(result,wide_local+2u128);
    aligned(&wide(7u64,9u64,wide_local));
    let values=[17u64,29u64];let tail=[29u64];assert_eq!(fat_odd(13u64,&values),2usize);assert_eq!(fat_even(&values,&tail),3usize);
    let mut order=0u64;assert_eq!(ordered(note(&mut order,1u64),fat_odd(13u64,&values),note(&mut order,2u64)),2usize);assert_eq!(order,12u64);
    let returned=recursive(3u64,wide_local);aligned(&returned);assert_eq!(returned,wide_local+3u128);
    let marker=Marker{tag:43u64};let object:&dyn Probe=&marker;let got=dispatch(object,59u64);aligned(&got);assert_eq!(got,101u128);
    println!("wide frame boundaries");
}
"#,
        "physical_wide_frames_arrays_nested_calls_recursion_traits_and_drop_match_stock",
    );
}

#[test]
fn physical_nominal_wide_field_exact_wrong_code_regression() {
    physical_matches_stock(
        r#"
struct S { a: u8, b: u128 }
fn observe(s: &S) { println!("{} {}", (&s.b as *const u128 as usize) % 16, s.b); }
fn main() { let value = S { a: 1, b: 7 }; observe(&value); assert_eq!(value.a, 1); }
"#,
        "nominal-wide-exact",
    );
}

#[test]
fn physical_nominal_tuple_enum_fields_copies_owners_and_drop_match_stock() {
    physical_matches_stock(
        r#"
#[derive(Clone, Copy)]
struct Wide { head: u8, value: u128, tail: u8 }
#[derive(Clone, Copy)]
struct Nest { head: u8, inner: Wide, tail: (u8, u128, u8) }
fn aligned(value: &u128) { assert_eq!(value as *const u128 as usize % 16usize, 0usize); }
fn inspect(value: &Wide) {
    aligned(&value.value);
    assert_eq!(value.head, 3u8);
    assert_eq!(value.value, 18446744073709551623u128);
    assert_eq!(value.tail, 5u8);
}
fn relay(value: Nest, guard: u64) -> Nest {
    assert_eq!(guard, 123u64);
    assert_eq!(&value as *const Nest as usize % 16usize, 0usize);
    assert_eq!(&value.inner as *const Wide as usize % 16usize, 0usize);
    inspect(&value.inner);
    aligned(&value.tail.1);
    assert_eq!(value.tail.0, 7u8);
    assert_eq!(value.tail.1, 36893488147419103241u128);
    assert_eq!(value.tail.2, 11u8);
    value
}
enum Choice { Empty, Data(u8, Wide, u128, u8) }
fn enum_relay(value: Choice, guard: u64) -> Choice { assert_eq!(guard, 97u64); value }
struct Dropped { head: u8, value: u128, tail: u8 }
impl Drop for Dropped {
    fn drop(&mut self) {
        aligned(&self.value);
        assert_eq!(self.head, 13u8);
        assert_eq!(self.value, 55340232221128654859u128);
        assert_eq!(self.tail, 17u8);
        println!("drop aligned");
    }
}
enum Owned { Empty, Data(u8, Dropped, u128, u8) }
fn owned(value: Owned) -> Owned { value }
fn main() {
    let before = 71u64;
    let value = Wide { head: 3, value: 18446744073709551623u128, tail: 5 };
    inspect(&value);
    assert_eq!(&value as *const Wide as usize % 16usize, 0usize);
    let nested = Nest { head: 19, inner: value, tail: (7, 36893488147419103241u128, 11) };
    let copied = relay(nested, 123);
    inspect(&nested.inner);
    inspect(&copied.inner);
    assert_eq!(copied.head, 19u8);
    let array = [value; 3];
    let dynamic = 1usize;
    inspect(&array[0]); inspect(&array[dynamic]); inspect(&array[2]);
    let boxed = Box::new(copied); inspect(&boxed.inner); aligned(&boxed.tail.1);
    let shared = std::rc::Rc::new(value); let cloned = shared.clone();
    inspect(&*shared); inspect(&*cloned);
    let mut values = Vec::new(); values.push(value); values.push(value); values.push(value);
    inspect(&values[0]); inspect(&values[dynamic]); inspect(&values[2]);
    match enum_relay(Choice::Data(23, value, 73786976294838206477u128, 29), 97) {
        Choice::Empty => panic!("wrong tag"),
        Choice::Data(ref head, ref inner, ref wide, ref tail) => {
            assert_eq!(*head, 23u8); inspect(inner); aligned(wide);
            assert_eq!(*wide, 73786976294838206477u128); assert_eq!(*tail, 29u8);
        }
    }
    match enum_relay(Choice::Empty, 97) { Choice::Empty => {}, Choice::Data(_,_,_,_) => panic!("wrong tag") }
    {
        let aggregate = owned(Owned::Data(31, Dropped { head: 13, value: 55340232221128654859u128, tail: 17 }, 41u128, 43));
        match &aggregate {
            Owned::Empty => panic!("wrong owner tag"),
            Owned::Data(head, inner, wide, tail) => {
                assert_eq!(*head, 31u8); aligned(&inner.value); aligned(wide);
                assert_eq!(*wide, 41u128); assert_eq!(*tail, 43u8);
            }
        }
    }
    {
        let boxed_drop = Box::new(Dropped { head: 13, value: 55340232221128654859u128, tail: 17 });
        // Drop observes the wide field in its original Box allocation.
    }
    {
        let shared_drop = std::rc::Rc::new(Dropped { head: 13, value: 55340232221128654859u128, tail: 17 });
        let _cloned_drop = shared_drop.clone();
        // The final Rc owner must run Drop at the original aligned address.
    }
    {
        let mut dropped_values = Vec::new();
        dropped_values.push(Dropped { head: 13, value: 55340232221128654859u128, tail: 17 });
        dropped_values.push(Dropped { head: 13, value: 55340232221128654859u128, tail: 17 });
        // Both Vec elements run Drop in place after growth and transport.
    }
    let _empty = owned(Owned::Empty);
    let after = 73u64;
    assert_eq!(before, 71u64); assert_eq!(after, 73u64);
    println!("nominal tuple enum aligned");
}

"#,
        "nominal-wide-transport",
    );
}

#[test]
fn physical_u16_byte_reinterpretation_matches_stock() {
    physical_matches_stock(
        r#"fn main(){let a=[257u16,513u16];let p=&a[0]as *const u16;let bytes=p as *const u8;unsafe{println!("{} {} {} {}",*bytes,*bytes.add(1usize),*bytes.add(2usize),*bytes.add(3usize));}}"#,
        "physical_u16_byte_reinterpretation_matches_stock",
    );
}

#[test]
fn alignment_domain_recursive_wrappers_match_stock() {
    physical_matches_stock(
        r#"
struct Pair<T>{left:T,right:[u16;3]}
enum Ignored{A,B}
fn left<T,U>(n:usize)->usize{if n==0usize{std::mem::align_of::<(T,[U;2])>()}else{right::<U,T>(n-1usize)}}
fn right<T,U>(n:usize)->usize{left::<U,T>(n)}
fn wrap<T>()->usize{std::mem::align_of::<Pair<T>>()}
fn unused_parameter<T>(n:usize)->usize{n}
fn raw_alignment<T:?Sized>()->usize{std::mem::align_of::<*const T>()}
fn grow<T>(n:usize)->usize{if n==0usize{std::mem::align_of::<T>()}else{grow::<[T;2]>(n-1usize)}}
fn main(){println!("{} {} {} {} {}",left::<u8,u32>(4usize),wrap::<u128>(),std::mem::align_of::<&Ignored>(),unused_parameter::<Ignored>(9usize),raw_alignment::<Ignored>());}
"#,
        "alignment-domain-recursive",
    );
}

#[test]
fn alignment_domain_closed_unsupported_wrappers_are_rejected_during_check() {
    for (tag, source) in [
        (
            "generic-unsized-raw",
            "fn raw<T:?Sized>()->usize{std::mem::align_of::<*const T>()} fn unused()->usize{raw::<str>()} fn main(){}",
        ),
        (
            "nested-enum",
            "enum E{A,B} struct Wrap<T>{value:[T;2]} fn inner<T>()->usize{std::mem::align_of::<Wrap<T>>()} fn outer<T>()->usize{inner::<(u8,T)>()} fn unused()->usize{outer::<E>()} fn main(){}",
        ),
        (
            "recursive-enum",
            "enum E{A,B} fn a<T,U>(n:usize)->usize{if n==0usize{std::mem::align_of::<(T,[U;2])>()}else{b::<U,T>(n-1usize)}} fn b<T,U>(n:usize)->usize{a::<U,T>(n)} fn unused()->usize{a::<u8,E>(0usize)} fn main(){}",
        ),
        (
            "captured-closure",
            "fn alignment<T>(_:T)->usize{std::mem::align_of::<T>()} fn unused()->usize{let n=3u8;alignment(||n)} fn main(){}",
        ),
        (
            "trait-dispatch-enum",
            "enum E{A,B}struct W<T>(T);trait Inspect{fn width(&self)->usize;}impl<T> Inspect for W<T>{fn width(&self)->usize{std::mem::align_of::<T>()}}fn invoke<T:Inspect>(x:T)->usize{x.width()}fn unused()->usize{invoke(W(E::A))}fn main(){}",
        ),
        (
            "virtual-constructor-enum",
            "enum E{A,B}struct W<T>(T);trait Inspect{fn width(&self)->usize;}impl<T> Inspect for W<T>{fn width(&self)->usize{std::mem::align_of::<T>()}}fn unused()->usize{let v=W(E::A);let p:&dyn Inspect=&v;p.width()}fn main(){}",
        ),
        (
            "opaque-bound-enum",
            "enum E{A,B}struct W<T>(T);trait Inspect{fn width(&self)->usize;}impl<T> Inspect for W<T>{fn width(&self)->usize{std::mem::align_of::<T>()}}fn make()->impl Inspect{W(E::A)}fn unused()->usize{make().width()}fn main(){}",
        ),
        (
            "opaque-method-own-enum",
            "enum E{A,B}struct V;trait Inspect{fn width<T>(&self)->usize;}impl Inspect for V{fn width<T>(&self)->usize{std::mem::align_of::<T>()}}fn make()->impl Inspect{V}fn unused()->usize{make().width::<E>()}fn main(){}",
        ),
        (
            "implicit-drop-enum",
            "enum E{A,B}struct D<T>(T);impl<T> Drop for D<T>{fn drop(&mut self){let _n=std::mem::align_of::<T>();}}fn consume<T>(_x:T){}fn unused(){consume(D(E::A));}fn main(){}",
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
                let error = match compiler::compile(source.into(), workers, check) {
                    Ok(_) => panic!(
                        "{tag}: workers={workers},check={check}: unsupported target alignment must fail admission"
                    ),
                    Err(error) => error.to_string(),
                };
                assert!(
                    error.contains("unsupported") && error.contains("alignment"),
                    "{tag}: workers={workers},check={check}: {error}"
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn alignment_domain_dispatch_projection_and_cleanup_match_stock() {
    for (tag, source) in [
        (
            "trait-dispatch",
            r#"trait Inspect{fn width(&self)->usize;}impl Inspect for u16{fn width(&self)->usize{std::mem::align_of::<u16>()}}fn invoke<T:Inspect>(x:T)->usize{x.width()}fn main(){println!("{}",invoke(7u16));}"#,
        ),
        (
            "virtual-constructor",
            r#"struct V(u16);trait Inspect{fn width(&self)->usize;}impl Inspect for V{fn width(&self)->usize{std::mem::align_of::<u16>()}}fn invoke(x:&dyn Inspect)->usize{x.width()}fn main(){let v=V(7u16);println!("{}",invoke(&v));}"#,
        ),
        (
            "opaque-bound",
            r#"struct V(u16);trait Inspect{fn width(&self)->usize;}impl Inspect for V{fn width(&self)->usize{std::mem::align_of::<u16>()}}fn make()->impl Inspect{V(7u16)}fn main(){println!("{}",make().width());}"#,
        ),
        (
            "opaque-method-own",
            r#"struct V;trait Inspect{fn width<T>(&self)->usize;}impl Inspect for V{fn width<T>(&self)->usize{std::mem::align_of::<T>()}}fn make()->impl Inspect{V}fn main(){println!("{}",make().width::<u32>());}"#,
        ),
        (
            "associated-query",
            r#"trait Has{type Item;}impl Has for u16{type Item=u32;}fn width<T:Has>()->usize{std::mem::align_of::<T::Item>()}fn main(){println!("{}",width::<u16>());}"#,
        ),
        (
            "implicit-drop",
            r#"struct D<T>(T);impl<T> Drop for D<T>{fn drop(&mut self){println!("{}",std::mem::align_of::<T>());}}fn consume<T>(_x:T){}fn main(){consume(D(7u16));}"#,
        ),
    ] {
        physical_matches_stock(source, tag);
    }
}

#[test]
fn alignment_domain_forwarded_opaque_uses_source_declaration_protocol() {
    let source = "struct V;trait Inspect{fn width(&self)->usize;}impl Inspect for V{fn width(&self)->usize{std::mem::align_of::<u16>()}}fn make()->impl Inspect{V}fn forward()->impl Inspect{make()}fn main(){assert!(forward().width()==2usize);}";
    physical_matches_stock(source, "forwarded-opaque-domain-declaration");
}

#[test]
fn alignment_domain_check_keeps_closed_instance_and_codegen_nodes_undemanded() {
    let source = "fn a<T>(n:usize)->usize{if n==0usize{std::mem::align_of::<T>()}else{b::<T>(n-1usize)}} fn b<U>(n:usize)->usize{a::<U>(n)} fn growing<T>(n:usize)->usize{if n==0usize{std::mem::align_of::<T>()}else{growing::<[T;2]>(n-1usize)}} fn main(){let _n=a::<u16>(2usize);}";
    let opaque = "struct V;trait Inspect{fn width(&self)->usize;}impl Inspect for V{fn width(&self)->usize{std::mem::align_of::<u16>()}}fn make()->impl Inspect{V}fn main(){let _n=make().width();}";
    for (source, workers) in [source, opaque]
        .into_iter()
        .flat_map(|source| [1, 8].map(|workers| (source, workers)))
    {
        let checked = compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        for kind in [102, 103, 108, 137, 140, 142] {
            assert!(
                !checked.report.published_kind(kind),
                "CHECK demanded backend/instance kind {kind}"
            );
        }
        assert!(checked.report.published_kind(115));
        assert!(checked.report.published_kind(128));
    }
}

#[test]
fn alignment_domain_graph_reads_compact_headers_not_transitive_bodies() {
    use std::collections::HashMap;
    let source = "fn leaf<T>()->usize{std::mem::align_of::<T>()}fn caller<T>()->usize{leaf::<T>()}fn apply<F:Fn()->usize>(f:F)->usize{f()}fn main(){let _n=caller::<u16>();let _m=apply(||std::mem::align_of::<u16>());}";
    for workers in [1, 8] {
        let compiled = compiler::compile_observed_with_options(
            source.into(),
            workers,
            true,
            compiler::source::SourceOptions::default(),
        )
        .unwrap();
        let report = &compiled.report;
        assert!(report.capture_complete);
        let nodes: HashMap<_, _> = report
            .node_evaluations
            .iter()
            .zip(&report.node_ids)
            .map(|((key, _), id)| (*id, key))
            .collect();
        let mut tasks = HashMap::new();
        for event in &report.events {
            if let (Some(task), Some(node)) = (event.task, event.node) {
                tasks.entry(task).or_insert(node);
            }
        }
        let mut symbolic_summaries = 0;
        let mut closed_summaries = 0;
        for key in nodes.values() {
            if key.kind == 115 && key.params.starts_with(&[u64::MAX - 1, 2]) {
                symbolic_summaries += 1;
            }
            if key.kind == 128 && key.params.starts_with(&[u64::MAX - 2, 2]) {
                closed_summaries += 1;
            }
        }
        assert!(symbolic_summaries > 0 && closed_summaries > 0);
        for &(task, producer, _) in &report.dependency_edges {
            let consumer = nodes[&tasks[&task]];
            let dependency = nodes[&producer];
            let symbolic = consumer.kind == 115 && consumer.params.first() == Some(&(u64::MAX - 1));
            let closed = consumer.kind == 128 && consumer.params.first() == Some(&(u64::MAX - 2));
            if !(symbolic || closed) {
                continue;
            }
            let flavor = consumer.params[1];
            if flavor != 0 {
                assert_ne!(
                    dependency.kind, 110,
                    "topology/SCC producer directly read a TemplateBody: {consumer:?} -> {dependency:?}"
                );
            }
            if symbolic && flavor == 1 {
                assert!(
                    dependency.kind == 115 && dependency.params.starts_with(&[u64::MAX - 1, 0]),
                    "symbolic topology consumed more than compact owner headers: {dependency:?}"
                );
            }
            if symbolic && flavor == 2 {
                assert!(
                    dependency.kind == 115 && dependency.params.first() == Some(&(u64::MAX - 1)),
                    "symbolic SCC consumed non-domain input: {dependency:?}"
                );
            }
            if closed && flavor == 2 {
                assert!(
                    dependency.kind == 128 && dependency.params.first() == Some(&(u64::MAX - 2)),
                    "closed SCC consumed non-domain input: {dependency:?}"
                );
            }
        }
    }
}
