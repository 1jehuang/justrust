struct Item {
    value: i32,
}

enum Event {
    Value(Item),
    Empty,
}

fn values() -> [i32; 3] {
    [2, 3, 5]
}

fn relay(events: [Event; 2]) -> [Event; 2] {
    events
}

fn main() {
    let mut a = values();
    let old = a;
    a[1] = 7;
    let repeated = [a[1]; 3];
    let empty: [Item; 0] = [];
    let events = relay([Event::Value(Item { value: 9 }), Event::Empty]);
    let _ = events[0];
    let _ = empty;
    if old[1] + repeated[2] == 10 {
        println!("fixed arrays compiled correctly");
    }
}
