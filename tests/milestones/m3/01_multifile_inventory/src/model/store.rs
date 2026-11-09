use super::item::Item;
use std::collections::BTreeMap;
#[derive(Default)]
pub struct Store { pub(crate) items: BTreeMap<String, Item> }
impl Store {
    pub fn add(&mut self, it: Item) { self.items.insert(it.name.clone(), it); }
    pub fn take(&mut self, name: &str, n: u32) -> Result<u32, String> {
        let it = self.items.get_mut(name).ok_or_else(|| format!("no {name}"))?;
        if it.qty < n { return Err(format!("only {} {name} left", it.qty)); }
        it.qty -= n; Ok(it.qty)
    }
}
