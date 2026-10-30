//! bevy's `in_set`, over wgpu's handles: `Arc`s of each other, through which a type is reached
//! first near the top of a `Send` goal and again deep below it. The second time takes as many
//! nested goals again as the first: the new trait solver checks the depth a cached result needs
//! where rustc's old one doesn't, so the goal overflowed into an ambiguity, `Weight::Of` was not
//! known and `run` was left to rustc. rustc's solver proves such a goal again with twice the
//! recursion limit; rustc's old one takes `Near` alone to 34 links.

use std::sync::{Arc, Mutex};

pub trait Weight {
    type Of;
    fn weight() -> Self::Of;
}

impl<T: Send> Weight for T {
    type Of = usize;
    fn weight() -> usize {
        core::mem::size_of::<T>()
    }
}

macro_rules! chain {
    ($($name:ident => $next:ident),*) => {
        $(
            pub struct $name {
                pub next: Option<Arc<Mutex<$next>>>,
            }
        )*
    };
}

pub struct Leaf {
    pub n: usize,
}

chain!(
    Near0 => Near1,
    Near1 => Near2,
    Near2 => Near3,
    Near3 => Near4,
    Near4 => Near5,
    Near5 => Near6,
    Near6 => Near7,
    Near7 => Near8,
    Near8 => Near9,
    Near9 => Near10,
    Near10 => Near11,
    Near11 => Near12,
    Near12 => Near13,
    Near13 => Near14,
    Near14 => Near15,
    Near15 => Near16,
    Near16 => Near17,
    Near17 => Near18,
    Near18 => Near19,
    Near19 => Leaf
);

chain!(
    Far0 => Far1,
    Far1 => Far2,
    Far2 => Far3,
    Far3 => Far4,
    Far4 => Far5,
    Far5 => Far6,
    Far6 => Far7,
    Far7 => Far8,
    Far8 => Far9,
    Far9 => Far10,
    Far10 => Far11,
    Far11 => Far12,
    Far12 => Far13,
    Far13 => Far14,
    Far14 => Far15,
    Far15 => Far16,
    Far16 => Far17,
    Far17 => Far18,
    Far18 => Far19,
    Far19 => Near0
);

pub struct Root {
    pub near: Near0,
    pub far: Far0,
}

pub fn run() -> usize {
    <Root as Weight>::weight()
}
