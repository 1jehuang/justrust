mod model {
    pub struct Leaf {
        pub n: i32,
    }
    pub struct Packet {
        pub head: Leaf,
        pub tail: (i32, bool),
    }
}
use model::Leaf;
use model::Packet;

fn advance(value: Packet) -> Packet {
    Packet {
        tail: value.tail,
        head: Leaf {
            n: value.head.n + 1,
        },
    }
}
fn total(value: Packet) -> i32 {
    value.head.n + value.tail.0
}

fn main() {
    let mut packet = Packet {
        head: Leaf { n: 1 },
        tail: (7, true),
    };
    let mut i = 0;
    while i < 3 {
        packet = advance(packet);
        i = i + 1;
    }
    let previous = packet.head;
    packet.head = Leaf { n: previous.n + 10 };
    if total(packet) == 21 {
        println!("nominal structs and partial-move reinitialization compiled correctly");
    }
}
