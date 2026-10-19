// Native Hash/Hasher traits; #[derive(Hash)] generates impls.
pub trait Hasher {
    fn finish(&self) -> u64;
    fn write_u64(&mut self, value: u64);
}

// TODO(generic trait methods): `fn hash<H: Hasher>(&self, state: &mut H)`.
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
    fn write_u64(&mut self, value: u64) {
        let mut i = 0u32;
        while i < 8u32 {
            let mixed = self.state ^ ((value >> (i * 8u32)) & 255u64);
            // xorshift mix (no wrapping multiply needed)
            let a = mixed ^ (mixed << 13u32);
            let b = a ^ (a >> 7u32);
            self.state = b ^ (b << 17u32);
            i += 1u32;
        }
    }
}

pub trait Hash {
    fn hash(&self, state: &mut DefaultHasher);
}

impl Hash for i8 {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for i16 {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for i32 {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for i64 {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for isize {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for u8 {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for u16 {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for u32 {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for u64 {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for usize {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u64);
    }
}

impl Hash for bool {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(if *self { 1u64 } else { 0u64 });
    }
}

impl Hash for char {
    fn hash(&self, state: &mut DefaultHasher) {
        state.write_u64(*self as u32 as u64);
    }
}


impl<A: Hash> Hash for (A,) {
    fn hash(&self, state: &mut DefaultHasher) {
        <A as Hash>::hash(&(*self).0, &mut *state);
    }
}

impl<A: Hash, B: Hash> Hash for (A, B) {
    fn hash(&self, state: &mut DefaultHasher) {
        <A as Hash>::hash(&(*self).0, &mut *state);
        <B as Hash>::hash(&(*self).1, &mut *state);
    }
}

impl<A: Hash, B: Hash, C: Hash> Hash for (A, B, C) {
    fn hash(&self, state: &mut DefaultHasher) {
        <A as Hash>::hash(&(*self).0, &mut *state);
        <B as Hash>::hash(&(*self).1, &mut *state);
        <C as Hash>::hash(&(*self).2, &mut *state);
    }
}

impl<A: Hash, B: Hash, C: Hash, D: Hash> Hash for (A, B, C, D) {
    fn hash(&self, state: &mut DefaultHasher) {
        <A as Hash>::hash(&(*self).0, &mut *state);
        <B as Hash>::hash(&(*self).1, &mut *state);
        <C as Hash>::hash(&(*self).2, &mut *state);
        <D as Hash>::hash(&(*self).3, &mut *state);
    }
}
