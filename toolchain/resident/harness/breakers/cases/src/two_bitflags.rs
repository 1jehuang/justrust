//! onig declares several sets of flags in one module. `bitflags!` puts the type that holds
//! the bits of each in a `const _: () = { .. }`, so two types of one name and one path are
//! in the module, and the methods of the two must not be given one symbol.

macro_rules! flags {
    ($name:ident, $bits:ty, $all:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct $name(<$name as Holds>::Internal);
        const _: () = {
            #[derive(Debug, Clone, Copy, PartialEq)]
            pub struct InternalBitFlags($bits);
            impl InternalBitFlags {
                pub fn bits(&self) -> $bits {
                    self.0
                }
                pub fn all() -> Self {
                    Self($all)
                }
            }
            impl Holds for $name {
                type Internal = InternalBitFlags;
            }
            impl $name {
                pub fn bits(&self) -> $bits {
                    self.0.bits()
                }
                pub fn all() -> Self {
                    Self(InternalBitFlags::all())
                }
            }
        };
    };
}

pub trait Holds {
    type Internal;
}

flags!(Options, u32, 0x1ff);
flags!(Operators, u64, 0xffff_0000_0000);

pub fn run() -> (u32, u64) {
    (Options::all().bits(), Operators::all().bits())
}
