//! yansi keeps whether styling is on as a function behind an `AtomicPtr`, in a static whose
//! value is an associated constant. Const eval gives a function as an id of the evaluation
//! that made it. A constant read by another evaluation had the ids of its function pointers
//! given anew, and not those of a function cast to a raw pointer: the static held the id of
//! another table, and the call through it jumped to 1000.

use std::sync::atomic::{AtomicPtr, Ordering};

pub struct Condition(pub fn() -> bool);

impl Condition {
    pub const DEFAULT: Condition = Condition(Condition::yes);

    pub fn yes() -> bool {
        true
    }
}

pub struct AtomicCondition(AtomicPtr<()>);

impl AtomicCondition {
    pub const DEFAULT: AtomicCondition = AtomicCondition::from(Condition::DEFAULT);

    pub const fn from(value: Condition) -> Self {
        AtomicCondition(AtomicPtr::new(value.0 as *mut ()))
    }

    pub fn read(&self) -> bool {
        let condition =
            unsafe { Condition(core::mem::transmute::<*mut (), fn() -> bool>(self.0.load(Ordering::Acquire))) };
        (condition.0)()
    }
}

static ENABLED: AtomicCondition = AtomicCondition::DEFAULT;

fn seven() -> u32 {
    7
}

const RAW: *const () = seven as fn() -> u32 as *const ();
const PAIR: (*const (), u8) = (RAW, 1);

pub fn run() -> (bool, u32) {
    let seven = unsafe { core::mem::transmute::<*const (), fn() -> u32>(PAIR.0) };
    (ENABLED.read(), seven())
}
