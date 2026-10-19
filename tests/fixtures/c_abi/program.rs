#[repr(C)]
pub struct Small { pub a: u8, pub b: i32 }
#[repr(C)]
pub struct Mixed { pub a: f64, pub b: u32 }
#[repr(C)]
pub struct Large { pub a: u8, pub b: u64, pub c: f32 }
#[repr(C)]
pub struct Nested { pub a: u16, pub b: Small, pub c: [u8; 3] }
#[repr(C)]
pub struct Pair { pub a: u64, pub b: u64 }
#[repr(C)]
pub struct PackedFloats { pub data: [f32; 3], pub tag: u8 }
unsafe extern "C" {
    fn native_scalars(a: i8, b: u16, c: i32, d: u64, e: bool, f: f32, g: f64, h: u64, i: u64, j: u64) -> u64;
    fn native_floats(a:f64,b:f64,c:f64,d:f64,e:f64,f:f64,g:f64,h:f64,i:f64,j:f64)->f64;
    fn native_wide(a:i128,b:u64,c:i128,d:u64,e:i128)->i128;
    fn native_small(value:Small)->Small;
    fn native_mixed(value:Mixed)->Mixed;
    fn native_large(value:Large)->Large;
    fn native_nested(value:Nested)->Nested;
    fn native_rollback(a:u64,b:u64,c:u64,d:u64,e:u64,p:Pair,last:u64)->u64;
    fn native_packed(value:PackedFloats)->PackedFloats;
    fn native_add(a:i32,b:i32)->i32;
    fn native_roundtrip(value:i32)->i32;
    fn native_output();
    fn labs(value:isize)->isize;
}
#[unsafe(no_mangle)]
pub extern "C" fn jr_callback(value:i32)->i32 { unsafe { native_add(value, 9) } }
#[unsafe(no_mangle)]
pub extern "C" fn jr_small(value:Small)->Small { Small { a:value.a + 1, b:value.b - 2 } }
#[unsafe(no_mangle)]
pub extern "C" fn jr_mixed(value:Mixed)->Mixed { Mixed {a:value.a + 1.5,b:value.b + 7} }
#[unsafe(no_mangle)]
pub extern "C" fn jr_large(value:Large)->Large { Large {a:value.a + 1,b:value.b + 2,c:value.c + 0.5} }
#[unsafe(no_mangle)]
pub extern "C" fn jr_nested(value:Nested)->Nested { Nested {a:value.a + 1,b:Small{a:value.b.a + 2,b:value.b.b + 3},c:[value.c[0] + 1,value.c[1] + 2,value.c[2] + 3]} }
#[unsafe(no_mangle)]
pub extern "C" fn jr_wide(a:i128,b:u64,c:i128,d:u64,e:i128)->i128 {a + b as i128 + c + d as i128 + e}
#[unsafe(no_mangle)]
pub extern "C" fn jr_rollback(a:u64,b:u64,c:u64,d:u64,e:u64,p:Pair,last:u64)->u64 {a+b+c+d+e+p.a+p.b+last}
#[unsafe(no_mangle)]
pub extern "C" fn jr_floats(a:f64,b:f64,c:f64,d:f64,e:f64,f:f64,g:f64,h:f64,i:f64,j:f64)->f64 {a+b+c+d+e+f+g+h+i+j}
#[unsafe(no_mangle)]
pub extern "C" fn jr_packed(value:PackedFloats)->PackedFloats {PackedFloats{data:[value.data[0]+1.0,value.data[1]+2.0,value.data[2]+3.0],tag:value.tag+1}}
fn main() {
    unsafe {
        assert_eq!(native_scalars(-7,65000,-123456,0xfedcba9876543210u64,true,1.5f32,2.25f64,8,9,10),12345);
        assert!(native_floats(1.0,2.0,3.0,4.0,5.0,6.0,7.0,8.0,9.0,10.0)==55.0);
        assert_eq!(native_wide(18446744073709551617i128,2,-18446744073709551616i128,3,5),11);
        let small=native_small(Small{a:7,b:-1234}); assert_eq!(small.a,8);assert_eq!(small.b,-1236);
        let mixed=native_mixed(Mixed{a:2.5,b:9});assert!(mixed.a==4.0);assert_eq!(mixed.b,16);
        let large=native_large(Large{a:3,b:123,c:2.5});assert_eq!(large.a,4);assert_eq!(large.b,125);assert!(large.c==3.0);
        let nested=native_nested(Nested{a:10,b:Small{a:20,b:-30},c:[1,2,3]});assert_eq!(nested.a,11);assert_eq!(nested.b.a,22);assert_eq!(nested.b.b,-27);assert_eq!(nested.c[0],2);assert_eq!(nested.c[1],4);assert_eq!(nested.c[2],6);
        assert_eq!(native_rollback(1,2,3,4,5,Pair{a:6,b:7},8),36);
        let packed=native_packed(PackedFloats{data:[1.0,2.0,3.0],tag:4});assert!(packed.data[0]==2.0);assert!(packed.data[1]==4.0);assert!(packed.data[2]==6.0);assert_eq!(packed.tag,5);
        assert_eq!(native_roundtrip(20),29);assert_eq!(labs(-123),123);
    }
    println!("C ABI roundtrip passed");
    unsafe { native_output(); }
}
