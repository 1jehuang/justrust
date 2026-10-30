//! rkyv_derive expands `#[derive(rkyv::Deserialize)]` on an enum to
//! `impl Deserialize<E, D> for <E as Archive>::Archived`, matching
//! `Self::Variant { .. }` in `deserialize()`. Because the impl's `Self` type is
//! the projection `<E as Archive>::Archived`, resolving `Self::Variant` in a
//! pattern needs `Self` structurally normalized before checking for an enum ADT.

trait Archive {
    type Archived;
}

trait Deserialize<T> {
    fn deserialize(&self) -> T;
}

#[derive(Debug, PartialEq)]
pub enum Item {
    Unit,
    Tuple(u32),
    Named { code: u32 },
}

pub enum ArchivedItem {
    Unit,
    Tuple(u32),
    Named { code: u32 },
}

impl Archive for Item {
    type Archived = ArchivedItem;
}

impl Deserialize<Item> for <Item as Archive>::Archived {
    fn deserialize(&self) -> Item {
        match self {
            Self::Unit => Item::Unit,
            Self::Tuple(_0, ..) => Item::Tuple(*_0 + 1),
            Self::Named { code, .. } => Item::Named { code: *code + 2 },
        }
    }
}

pub fn run() -> (Item, Item, Item) {
    (
        ArchivedItem::Unit.deserialize(),
        ArchivedItem::Tuple(10).deserialize(),
        ArchivedItem::Named { code: 20 }.deserialize(),
    )
}
