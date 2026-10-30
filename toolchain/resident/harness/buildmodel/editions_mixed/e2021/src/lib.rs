struct Pair {
    name: String,
    count: u32,
}

pub fn run(n: u32) -> String {
    let pair = Pair { name: "2021".to_owned(), count: n };
    let name = pair.name;
    let count = move || pair.count + [1u32, 2].into_iter().sum::<u32>();
    format!("{name}:{}", count())
}
