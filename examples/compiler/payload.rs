struct Record { value: i32 }
enum Message {
    Empty,
    Pair(Record, (i32, bool)),
    Named { right: i32, left: Record },
}
enum Single { Only(Record, Record) }

fn make(n: i32) -> Message {
    if n == 0 { Message::Empty } else {
        Message::Pair(Record { value: n }, (7, true))
    }
}

fn consume(message: Message) -> i32 {
    match message {
        Message::Empty => 0,
        Message::Pair(record, (n, yes)) => {
            if yes { record.value + n } else { 0 }
        },
        Message::Named { left, right } => left.value + right,
    }
}

fn main() {
    let pair = Single::Only(Record { value: 3 }, Record { value: 5 });
    let Single::Only(left, _) = pair;
    let Single::Only(_, right) = pair;
    let result = consume(make(left.value)) + right.value;
    let named = Message::Named { right: 11, left: Record { value: 13 } };
    if result == 15 && consume(named) == 24 && consume(make(0)) == 0 {
        println!("payload enums compiled correctly");
    }
}
