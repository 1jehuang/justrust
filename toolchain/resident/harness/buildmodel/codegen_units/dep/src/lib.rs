pub fn describe(words: &[&str]) -> String {
    let total: usize = words.iter().map(|w| w.len()).sum();
    format!("{} words, {} bytes, longest {:?}", words.len(), total, words.iter().max_by_key(|w| w.len()))
}
