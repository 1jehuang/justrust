#[derive(Debug, Clone, Copy, PartialEq)]
enum Light {
    Red,
    Green,
    Yellow,
}

#[derive(Debug)]
enum Event {
    Timer,
    Emergency,
    Reset,
}

impl Light {
    fn next(self, event: &Event) -> Light {
        match (self, event) {
            (_, Event::Emergency) => Light::Red,
            (_, Event::Reset) => Light::Red,
            (Light::Red, Event::Timer) => Light::Green,
            (Light::Green, Event::Timer) => Light::Yellow,
            (Light::Yellow, Event::Timer) => Light::Red,
        }
    }

    fn duration(self) -> u32 {
        match self {
            Light::Red => 30,
            Light::Green => 25,
            Light::Yellow => 5,
        }
    }
}

fn main() {
    let events = [Event::Timer, Event::Timer, Event::Emergency, Event::Timer, Event::Timer, Event::Timer, Event::Reset];
    let mut state = Light::Red;
    let mut total = 0;
    for e in events.iter() {
        let next = state.next(e);
        println!("{:?} --{:?}--> {:?}", state, e, next);
        total += next.duration();
        state = next;
    }
    println!("total time: {}", total);
    println!("ended red: {}", state == Light::Red);
}
