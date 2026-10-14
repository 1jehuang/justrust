use std::cell::RefCell;
thread_local! { static T: RefCell<Vec<u64>> = RefCell::new(Vec::new()); }
pub fn record(x: u64) {
    T.with(|t| {
        let mut t = t.borrow_mut();
        t.push(x); // ANCHOR
        if x > 100 {
            eprintln!("slow {}", x);
        }
    });
}
