// Runtime type identity is a compiler-owned observation of a canonical type.
// Equality, ordering, and hashing remain ordinary library source operations.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeId {
    value: usize,
}

impl TypeId {
    pub fn of<T: 'static + ?Sized>() -> TypeId {
        TypeId {
            value: type_id::<T>(),
        }
    }
}

fn type_id<T: 'static + ?Sized>() -> usize {
    __justrust_type_id!(T)
}

pub trait Any: 'static {
    fn type_id(&self) -> TypeId;
}

impl<T: 'static + ?Sized> Any for T {
    fn type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }
}

impl dyn Any {
    pub fn is<T: Any + 'static>(&self) -> bool {
        self.type_id() == TypeId::of::<T>()
    }

    pub fn downcast_ref<T: Any + 'static>(&self) -> Option<&T> {
        if self.is::<T>() {
            // The successful canonical identity comparison proves the erased
            // data pointer has exactly T's layout. Borrowing does not transfer
            // ownership and the result lifetime is tied to the receiver.
            unsafe { Some(&*(self as *const dyn Any as *const T)) }
        } else {
            None
        }
    }

    pub fn downcast_mut<T: Any + 'static>(&mut self) -> Option<&mut T> {
        if self.is::<T>() {
            unsafe { Some(&mut *(self as *mut dyn Any as *mut T)) }
        } else {
            None
        }
    }
}
