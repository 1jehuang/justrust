use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Category {
    Tool,
    Food,
    Part,
}

#[derive(Debug, Clone)]
struct Item {
    id: u32,
    name: String,
    category: Category,
    qty: u32,
    price_cents: u64,
}

#[derive(Debug)]
enum InvError {
    NotFound(u32),
    Insufficient { id: u32, have: u32, want: u32 },
    Duplicate(u32),
}

impl fmt::Display for InvError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            InvError::NotFound(id) => write!(f, "item {} not found", id),
            InvError::Insufficient { id, have, want } => write!(f, "item {}: have {}, want {}", id, have, want),
            InvError::Duplicate(id) => write!(f, "item {} already exists", id),
        }
    }
}

trait Observer {
    fn notify(&self, msg: &str);
}

struct Log {
    lines: RefCell<Vec<String>>,
}

impl Observer for Log {
    fn notify(&self, msg: &str) {
        self.lines.borrow_mut().push(msg.to_string());
    }
}

struct Inventory {
    items: HashMap<u32, Item>,
    observers: Vec<Rc<dyn Observer>>,
}

impl Inventory {
    fn new() -> Self {
        Inventory { items: HashMap::new(), observers: Vec::new() }
    }

    fn emit(&self, msg: String) {
        for o in &self.observers {
            o.notify(&msg);
        }
    }

    fn add(&mut self, item: Item) -> Result<(), InvError> {
        if self.items.contains_key(&item.id) {
            return Err(InvError::Duplicate(item.id));
        }
        self.emit(format!("added {} x{}", item.name, item.qty));
        self.items.insert(item.id, item);
        Ok(())
    }

    fn take(&mut self, id: u32, want: u32) -> Result<u64, InvError> {
        let item = self.items.get_mut(&id).ok_or(InvError::NotFound(id))?;
        if item.qty < want {
            return Err(InvError::Insufficient { id, have: item.qty, want });
        }
        item.qty -= want;
        let cost = item.price_cents * want as u64;
        let msg = format!("took {} x{} for {}", item.name, want, cost);
        self.emit(msg);
        Ok(cost)
    }

    fn value_by_category(&self) -> Vec<(Category, u64)> {
        let mut totals: HashMap<Category, u64> = HashMap::new();
        for it in self.items.values() {
            *totals.entry(it.category).or_insert(0) += it.price_cents * it.qty as u64;
        }
        let mut v: Vec<_> = totals.into_iter().collect();
        v.sort();
        v
    }
}

fn dollars(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

fn main() {
    let log = Rc::new(Log { lines: RefCell::new(Vec::new()) });
    let mut inv = Inventory::new();
    inv.observers.push(log.clone());
    let seed = vec![
        Item { id: 1, name: "hammer".into(), category: Category::Tool, qty: 5, price_cents: 1299 },
        Item { id: 2, name: "apple".into(), category: Category::Food, qty: 100, price_cents: 45 },
        Item { id: 3, name: "bolt".into(), category: Category::Part, qty: 500, price_cents: 12 },
        Item { id: 4, name: "wrench".into(), category: Category::Tool, qty: 3, price_cents: 899 },
    ];
    for it in seed.iter().cloned() {
        inv.add(it).unwrap();
    }
    if let Err(e) = inv.add(seed[0].clone()) {
        println!("error: {}", e);
    }
    let orders = [(1, 2), (2, 30), (3, 1000), (9, 1), (4, 3)];
    let mut revenue = 0;
    for (id, qty) in orders {
        match inv.take(id, qty) {
            Ok(c) => revenue += c,
            Err(e) => println!("error: {}", e),
        }
    }
    println!("revenue: {}", dollars(revenue));
    for (cat, v) in inv.value_by_category() {
        println!("{:?}: {}", cat, dollars(v));
    }
    let mut empty: Vec<&Item> = inv.items.values().filter(|i| i.qty == 0).collect();
    empty.sort_by_key(|i| i.id);
    let names: Vec<&str> = empty.iter().map(|i| i.name.as_str()).collect();
    println!("out of stock: {:?}", names);
    println!("log has {} entries (rc count {})", log.lines.borrow().len(), Rc::strong_count(&log));
    for line in log.lines.borrow().iter() {
        println!("  {}", line);
    }
}
