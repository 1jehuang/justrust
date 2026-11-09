use crate::model::store::Store;
use std::fmt::Write;
pub fn render(s: &Store) -> String {
    let mut out = String::new();
    for it in s.items.values() { writeln!(out, "{:<6}{:>5}{:>9.2}", it.name, it.qty, it.value()).unwrap(); }
    writeln!(out, "total {:.2}", s.items.values().map(|i| i.value()).sum::<f64>()).unwrap();
    out
}
