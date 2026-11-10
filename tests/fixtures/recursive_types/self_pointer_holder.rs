use std::rc::{Rc, Weak};

// A generic pointer wrapper with its own destructor, applied to the type
// that contains it: the closing edge of the cycle is a type argument.
struct Link<T> {
    target: *mut T,
}

impl<T> Drop for Link<T> {
    fn drop(&mut self) {
        println!("link {}", self.target as usize == 0usize);
    }
}

struct Holder {
    id: u32,
    me: Link<Holder>,
}

struct Watched {
    id: u32,
    me: Weak<Watched>,
}

impl Drop for Watched {
    fn drop(&mut self) {
        println!("watched {} {}", self.id, self.me.upgrade().is_none());
    }
}

fn main() {
    let plain = Holder {
        id: 1,
        me: Link {
            target: 0usize as *mut Holder,
        },
    };
    println!("plain {}", plain.id);
    let shared = Rc::new(Holder {
        id: 2,
        me: Link {
            target: 0usize as *mut Holder,
        },
    });
    println!("shared {} {}", shared.id, Rc::strong_count(&shared));
    drop(shared);
    let watched = Rc::new(Watched {
        id: 3,
        me: Weak::new(),
    });
    println!(
        "watched {} {} {}",
        watched.id,
        watched.me.upgrade().is_none(),
        Rc::weak_count(&watched)
    );
    drop(watched);
    println!("end");
}
