//! A dependency rustc builds, with the conversion trait of nsh-core's `ui::reactive` (ra-jit
//! #224): a generic parameter, impls for concrete types and handles, and a blanket impl for
//! closures whose return type converts into the parameter.

use std::cell::RefCell;
use std::rc::Rc;

pub struct Signal<T>(Rc<RefCell<T>>);

impl<T: Clone> Signal<T> {
    pub fn new(value: T) -> Self {
        Signal(Rc::new(RefCell::new(value)))
    }

    pub fn get(&self) -> T {
        self.0.borrow().clone()
    }

    pub fn set(&self, value: T) {
        *self.0.borrow_mut() = value;
    }
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Signal(self.0.clone())
    }
}

pub enum Reactive<T> {
    Static(T),
    Bound(Rc<dyn Fn() -> T>),
}

impl<T: Clone> Reactive<T> {
    pub fn get(&self) -> T {
        match self {
            Self::Static(v) => v.clone(),
            Self::Bound(f) => f(),
        }
    }
}

pub trait IntoReactive<T> {
    fn into_reactive(self) -> Reactive<T>;
}

impl<T> IntoReactive<T> for Reactive<T> {
    fn into_reactive(self) -> Reactive<T> {
        self
    }
}

impl<T: 'static, U: Into<T>, F: Fn() -> U + 'static> IntoReactive<T> for F {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Bound(Rc::new(move || self().into()))
    }
}

impl<T: Clone + 'static> IntoReactive<T> for Signal<T> {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Bound(Rc::new(move || self.get()))
    }
}

impl IntoReactive<String> for String {
    fn into_reactive(self) -> Reactive<String> {
        Reactive::Static(self)
    }
}

impl IntoReactive<u64> for u64 {
    fn into_reactive(self) -> Reactive<u64> {
        Reactive::Static(self)
    }
}

impl IntoReactive<String> for &str {
    fn into_reactive(self) -> Reactive<String> {
        Reactive::Static(self.to_owned())
    }
}

/// What a widget does with the value it is given, monomorphized by its caller.
pub fn bind_value<V: IntoReactive<String>>(value: V) -> Reactive<String> {
    value.into_reactive()
}
