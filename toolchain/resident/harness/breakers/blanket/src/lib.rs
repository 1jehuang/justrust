//! downcast-rs is this alone: a trait and its `impl<T>` for every type. rustc builds no code
//! of such a crate, and in a test that uses none of it no symbol names the crate either. A
//! crate rustc built nothing of is one ra-jit names the functions of: it compiled `as_any` with
//! nothing for `T`. `cratetest.py harness/breakers/blanket` runs it.

use std::any::Any;

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
