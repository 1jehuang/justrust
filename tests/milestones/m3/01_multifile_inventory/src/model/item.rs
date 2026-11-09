#[derive(Debug, Clone)]
pub struct Item { pub name: String, pub qty: u32, pub unit: f64 }
impl Item {
    pub fn new(name: &str, qty: u32, unit: f64) -> Self { Item { name: name.to_string(), qty, unit } }
    pub fn value(&self) -> f64 { self.qty as f64 * self.unit }
}
