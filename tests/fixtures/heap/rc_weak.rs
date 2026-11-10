use std::rc::{Rc, Weak};

struct Loud {
    id: u32,
}

impl Drop for Loud {
    fn drop(&mut self) {
        println!("drop {}", self.id);
    }
}

trait Speak {
    fn id(&self) -> u32;
}

impl Speak for Loud {
    fn id(&self) -> u32 {
        self.id
    }
}

// The Weak field is released by the payload's own drop glue, which runs
// while the last strong owner is being destroyed.
struct Watcher {
    id: u32,
    target: Weak<Loud>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        println!("watcher {} sees {}", self.id, self.target.strong_count());
    }
}

fn live_weaks_outlive_the_value() {
    let rc = Rc::new(Loud { id: 1 });
    let w1 = Rc::downgrade(&rc);
    let w2 = w1.clone();
    println!(
        "{} {} {} {}",
        Rc::strong_count(&rc),
        Rc::weak_count(&rc),
        w1.strong_count(),
        w1.weak_count()
    );
    println!("{:?} {}", w1, w1.ptr_eq(&w2));
    let up = w1.upgrade().unwrap();
    println!("up {} strong={}", up.id, Rc::strong_count(&rc));
    drop(up);
    drop(rc);
    println!(
        "after rc: {} {} {}",
        w1.upgrade().is_none(),
        w1.strong_count(),
        w1.weak_count()
    );
    drop(w1);
    println!("one weak left {}", w2.strong_count());
    drop(w2);
}

fn weak_dropped_before_the_last_strong() {
    let rc = Rc::new(Loud { id: 2 });
    {
        let w = Rc::downgrade(&rc);
        println!("inner {}", Rc::weak_count(&rc));
        let _ = w;
    }
    println!("outer {}", Rc::weak_count(&rc));
    drop(rc);
}

fn detached() {
    let n: Weak<Loud> = Weak::new();
    let d = <Weak<Loud> as Default>::default();
    let c = n.clone();
    println!(
        "{} {} {} {} {:?}",
        n.upgrade().is_none(),
        n.strong_count(),
        n.weak_count(),
        c.ptr_eq(&d),
        d
    );
}

fn weak_released_inside_a_destructor() {
    let target = Rc::new(Loud { id: 7 });
    let watcher = Rc::new(Watcher {
        id: 8,
        target: Rc::downgrade(&target),
    });
    let watching = Rc::downgrade(&watcher);
    println!(
        "{} {}",
        Rc::weak_count(&target),
        watching.upgrade().map(|w| w.id).unwrap_or(0)
    );
    drop(target);
    drop(watcher);
    println!("gone {}", watching.upgrade().is_none());
}

fn erased_owner_shares_the_counts() {
    let typed = Rc::new(Loud { id: 4 });
    let weak = Rc::downgrade(&typed);
    let object: Rc<dyn Speak> = typed;
    println!(
        "dyn {} {} {}",
        object.id(),
        Rc::strong_count(&object),
        Rc::weak_count(&object)
    );
    drop(object);
    println!("dyn dead {}", weak.upgrade().is_none());
    drop(weak);
    let typed = Rc::new(Loud { id: 5 });
    let object: Rc<dyn Speak> = typed.clone();
    let weak = Rc::downgrade(&typed);
    drop(typed);
    println!("dyn alive {}", weak.upgrade().map(|l| l.id).unwrap_or(0));
    drop(weak);
    drop(object);
}

fn many_observers() {
    let mut list: Vec<Weak<Loud>> = Vec::new();
    {
        let keep = Rc::new(Loud { id: 6 });
        for _ in 0..3 {
            list.push(Rc::downgrade(&keep));
        }
        println!("{}", Rc::weak_count(&keep));
    }
    println!("{}", list.iter().filter(|w| w.upgrade().is_none()).count());
}

fn main() {
    live_weaks_outlive_the_value();
    weak_dropped_before_the_last_strong();
    detached();
    weak_released_inside_a_destructor();
    erased_owner_shares_the_counts();
    many_observers();
    // Every count allocation is created and released once per round.
    let mut round = 0;
    let mut alive = 0usize;
    while round < 2000 {
        let rc = Rc::new(round);
        let weak = Rc::downgrade(&rc);
        if round % 2 == 0 {
            drop(rc);
            alive += weak.strong_count();
        } else {
            drop(weak);
            alive += Rc::weak_count(&rc);
        }
        round += 1;
    }
    println!("end {}", alive);
}
