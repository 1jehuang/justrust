pub fn run(n: u32) -> String {
    let items = [n, n + 1];
    #[allow(array_into_iter)]
    let total: u32 = items.into_iter().map(|it: &u32| *it).sum();
    format!("2018:{}", total)
}
