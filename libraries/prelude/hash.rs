// Native Hash/Hasher traits in std's shape; #[derive(Hash)] generates impls.
// DefaultHasher's mixing is our own (std's output is unspecified, no SipHash
// claim). Provided defaults follow core::hash: each unsigned write_* calls
// `write` exactly once with the value's native-endian bytes (little-endian on
// the supported x86-64 target), and each signed write_* delegates to its
// unsigned counterpart (isize to usize). usize never routes through write_u64.
fn le_bytes(value: u128, width: usize) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0usize;
    while i < width {
        out[i] = (value >> ((i as u32) * 8u32)) as u8;
        i += 1usize;
    }
    out
}

pub trait Hasher {
    fn finish(&self) -> u64;
    fn write(&mut self, bytes: &[u8]);
    fn write_u8(&mut self, i: u8) {
        self.write(&[i]);
    }
    fn write_u16(&mut self, i: u16) {
        let b = le_bytes(i as u128, 2usize);
        self.write(&[b[0], b[1]]);
    }
    fn write_u32(&mut self, i: u32) {
        let b = le_bytes(i as u128, 4usize);
        self.write(&[b[0], b[1], b[2], b[3]]);
    }
    fn write_u64(&mut self, i: u64) {
        let b = le_bytes(i as u128, 8usize);
        self.write(&[b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
    }
    fn write_u128(&mut self, i: u128) {
        let b = le_bytes(i, 16usize);
        self.write(&b);
    }
    fn write_usize(&mut self, i: usize) {
        let b = le_bytes(i as u128, 8usize);
        self.write(&[b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
    }
    fn write_i8(&mut self, i: i8) {
        self.write_u8(i as u8);
    }
    fn write_i16(&mut self, i: i16) {
        self.write_u16(i as u16);
    }
    fn write_i32(&mut self, i: i32) {
        self.write_u32(i as u32);
    }
    fn write_i64(&mut self, i: i64) {
        self.write_u64(i as u64);
    }
    fn write_i128(&mut self, i: i128) {
        self.write_u128(i as u128);
    }
    fn write_isize(&mut self, i: isize) {
        self.write_usize(i as usize);
    }
}

pub struct DefaultHasher {
    state: u64,
}
impl DefaultHasher {
    pub fn new() -> DefaultHasher {
        DefaultHasher { state: 14695981039346656037u64 }
    }
}
impl Hasher for DefaultHasher {
    fn finish(&self) -> u64 {
        self.state
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut i = 0usize;
        while i < bytes.len() {
            let mixed = self.state ^ (bytes[i] as u64);
            // xorshift mix (no wrapping multiply needed)
            let a = mixed ^ (mixed << 13u32);
            let b = a ^ (a >> 7u32);
            self.state = b ^ (b << 17u32);
            i += 1usize;
        }
    }
}

pub trait Hash {
    fn hash<H: Hasher>(&self, state: &mut H);

    fn hash_slice<H: Hasher>(data: &[Self], state: &mut H) where Self: Sized {
        let mut i = 0usize;
        while i < data.len() {
            <Self as Hash>::hash(&data[i], state);
            i += 1usize;
        }
    }
}

pub trait BuildHasher {
    type Hasher: Hasher;

    fn build_hasher(&self) -> Self::Hasher;

    fn hash_one<T: Hash>(&self, value: T) -> u64
    where
        Self: Sized,
        Self::Hasher: Hasher,
    {
        let mut state = self.build_hasher();
        value.hash(&mut state);
        state.finish()
    }
}

// The function marker records H without owning it or inheriting its auto traits.
// Construction and cloning never construct a hasher. Only build_hasher does.
pub struct BuildHasherDefault<H> {
    marker: core::marker::PhantomData<fn() -> H>,
}

impl<H> BuildHasherDefault<H> {
    pub const fn new() -> BuildHasherDefault<H> {
        BuildHasherDefault { marker: core::marker::PhantomData }
    }
}

impl<H> super::default::Default for BuildHasherDefault<H> {
    fn default() -> BuildHasherDefault<H> {
        BuildHasherDefault::<H>::new()
    }
}

impl<H> Clone for BuildHasherDefault<H> {
    fn clone(&self) -> BuildHasherDefault<H> {
        BuildHasherDefault::<H>::new()
    }
}

impl<H> super::cmp::PartialEq for BuildHasherDefault<H> {
    fn eq(&self, _other: &BuildHasherDefault<H>) -> bool {
        true
    }
}

impl<H> super::cmp::Eq for BuildHasherDefault<H> {}

impl<H> super::fmt::Debug for BuildHasherDefault<H> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.write_str("BuildHasherDefault")
    }
}

impl<H: super::default::Default + Hasher> BuildHasher for BuildHasherDefault<H> {
    type Hasher = H;

    fn build_hasher(&self) -> H {
        H::default()
    }
}

// Integer representations have no padding or uninitialized
// bytes. Their slice protocol writes the whole native-endian representation
// once, rather than calling a potentially overridden write_* per element.
// Only those primitive implementations call this private helper.
fn hash_primitive_slice<T, H: Hasher>(data: &[T], state: &mut H) {
    if data.len() == 0usize {
        state.write(&[]);
    } else {
        let pointer = &data[0] as *const T as *const u8;
        let length = data.len() * super::mem::size_of::<T>();
        let bytes = unsafe { super::slice::from_raw_parts::<u8>(pointer, length) };
        state.write(bytes);
    }
}

impl<T: Hash> Hash for [T] {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.len());
        T::hash_slice(self, state);
    }
}

impl<T: Hash> Hash for super::vec::Vec<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl<T: Hash + ?Sized> Hash for &T {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <T as Hash>::hash(*self, state)
    }
}

impl Hash for i8 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i8(*self);
    }
    fn hash_slice<H: Hasher>(data: &[i8], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for i16 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i16(*self);
    }
    fn hash_slice<H: Hasher>(data: &[i16], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for i32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(*self);
    }
    fn hash_slice<H: Hasher>(data: &[i32], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for i64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i64(*self);
    }
    fn hash_slice<H: Hasher>(data: &[i64], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for isize {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_isize(*self);
    }
    fn hash_slice<H: Hasher>(data: &[isize], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for u8 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u8(*self);
    }
    fn hash_slice<H: Hasher>(data: &[u8], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for u16 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u16(*self);
    }
    fn hash_slice<H: Hasher>(data: &[u16], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for u32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u32(*self);
    }
    fn hash_slice<H: Hasher>(data: &[u32], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for u64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(*self);
    }
    fn hash_slice<H: Hasher>(data: &[u64], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for usize {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(*self);
    }
    fn hash_slice<H: Hasher>(data: &[usize], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for u128 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u128(*self);
    }
    fn hash_slice<H: Hasher>(data: &[u128], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for i128 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i128(*self);
    }
    fn hash_slice<H: Hasher>(data: &[i128], state: &mut H) {
        hash_primitive_slice(data, state);
    }
}

impl Hash for bool {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u8(if *self { 1u8 } else { 0u8 });
    }
}

impl Hash for char {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u32(*self as u32);
    }
}


impl<A: Hash> Hash for (A,) {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <A as Hash>::hash(&(*self).0, &mut *state);
    }
}

impl<A: Hash, B: Hash> Hash for (A, B) {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <A as Hash>::hash(&(*self).0, &mut *state);
        <B as Hash>::hash(&(*self).1, &mut *state);
    }
}

impl<A: Hash, B: Hash, C: Hash> Hash for (A, B, C) {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <A as Hash>::hash(&(*self).0, &mut *state);
        <B as Hash>::hash(&(*self).1, &mut *state);
        <C as Hash>::hash(&(*self).2, &mut *state);
    }
}

impl<A: Hash, B: Hash, C: Hash, D: Hash> Hash for (A, B, C, D) {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <A as Hash>::hash(&(*self).0, &mut *state);
        <B as Hash>::hash(&(*self).1, &mut *state);
        <C as Hash>::hash(&(*self).2, &mut *state);
        <D as Hash>::hash(&(*self).3, &mut *state);
    }
}

/// std's default `BuildHasher` for HashMap/HashSet. Keys are fixed (no
/// per-process randomness); iteration order is unspecified either way.
pub struct RandomState {
    seed: u64,
}

impl Clone for RandomState {
    fn clone(&self) -> RandomState {
        RandomState { seed: self.seed }
    }
}

impl RandomState {
    pub fn new() -> RandomState {
        RandomState { seed: 0u64 }
    }
}

impl super::default::Default for RandomState {
    fn default() -> RandomState {
        RandomState::new()
    }
}

impl super::fmt::Debug for RandomState {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.write_str("RandomState { .. }")
    }
}

impl BuildHasher for RandomState {
    type Hasher = DefaultHasher;
    fn build_hasher(&self) -> DefaultHasher {
        DefaultHasher::new()
    }
}
