//! `std::borrow::Borrow`: a borrowed view of an owned key, used by map
//! lookups such as `HashMap<String, V>::get("key")`.
pub trait Borrow<Borrowed: ?Sized> {
    fn borrow(&self) -> &Borrowed;
}

impl<T> Borrow<T> for T {
    fn borrow(&self) -> &T {
        self
    }
}
