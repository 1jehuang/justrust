struct View<T> { ptr: *mut T, len: usize }
impl<T> View<T> {
    fn as_slice(&self) -> &[T] { unsafe { std::slice::from_raw_parts::<T>(self.ptr as *const T, self.len) } }
    fn as_mut_slice(&mut self) -> &mut [T] { unsafe { std::slice::from_raw_parts_mut::<T>(self.ptr, self.len) } }
}
struct Item { value: i32 }
fn pointer(p: *const i32) -> *const i32 { println!("pointer"); p }
fn length() -> usize { println!("length"); 2 }
fn main() {
    let mut data = [2, 3, 4];
    let mut view = View { ptr: &mut data as *mut [i32; 3] as *mut i32, len: 3 };
    let r = view.as_mut_slice();
    r[1] = 9;
    let s = view.as_slice();
    let n = s.len();
    let x = s[1];
    println!("{} {}", n, x);
    let raw = unsafe { std::slice::from_raw_parts::<i32>(pointer(&data as *const [i32; 3] as *const i32), length()) };
    let n = raw.len();
    println!("{}", n);
    let empty = unsafe { std::slice::from_raw_parts::<i32>(8usize as *const i32, 0) };
    let n = empty.len();
    println!("{}", n);
    let empty_mut = unsafe { std::slice::from_raw_parts_mut::<Item>(8usize as *mut Item, 0) };
    let n = empty_mut.len();
    println!("{}", n);
    let mut items = [Item { value: 7 }, Item { value: 8 }];
    let mut view = View { ptr: &mut items as *mut [Item; 2] as *mut Item, len: 2 };
    let shared = view.as_slice();
    let n = shared.len();
    let unique = view.as_mut_slice();
    let m = unique.len();
    println!("{} {}", n, m);
}
