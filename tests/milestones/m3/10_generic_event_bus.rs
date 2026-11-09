use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
trait Event: Any { fn name(&self) -> &'static str; fn as_any(&self) -> &dyn Any; }
#[derive(Debug)] struct Click { x: i32, y: i32 }
#[derive(Debug)] struct Key(char);
impl Event for Click { fn name(&self) -> &'static str { "click" } fn as_any(&self) -> &dyn Any { self } }
impl Event for Key { fn name(&self) -> &'static str { "key" } fn as_any(&self) -> &dyn Any { self } }
type Handler = Box<dyn Fn(&dyn Event) -> Option<String>>;
#[derive(Default)] struct Bus { subs: HashMap<&'static str, Vec<Handler>> }
impl Bus {
    fn on<E: Event + 'static>(&mut self, name: &'static str, f: impl Fn(&E) -> String + 'static) { self.subs.entry(name).or_default().push(Box::new(move |e| e.as_any().downcast_ref::<E>().map(&f))); }
    fn emit(&self, e: &dyn Event) -> Vec<String> { self.subs.get(e.name()).map(|hs| hs.iter().filter_map(|h| h(e)).collect()).unwrap_or_default() }
}
fn main() {
    let total = Rc::new(RefCell::new(0));
    let mut bus = Bus::default();
    let t = total.clone();
    bus.on::<Click>("click", move |c| { *t.borrow_mut() += c.x + c.y; format!("clicked {:?}", c) });
    bus.on::<Click>("click", |c| format!("quadrant {}", if c.x >= 0 { if c.y >= 0 { 1 } else { 4 } } else if c.y >= 0 { 2 } else { 3 }));
    bus.on::<Key>("key", |k| format!("key {:?} upper={}", k.0, k.0.to_ascii_uppercase()));
    let events: Vec<Box<dyn Event>> = vec![Box::new(Click { x: 3, y: -4 }), Box::new(Key('q')), Box::new(Click { x: -1, y: 2 })];
    for e in &events { for line in bus.emit(e.as_ref()) { println!("{line}"); } }
    println!("total={}", total.borrow());
}
