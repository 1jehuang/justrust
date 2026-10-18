fn is_palindrome(s: &str) -> bool {
    let cleaned: Vec<char> = s.chars().filter(|c| c.is_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
    let n = cleaned.len();
    for i in 0..n / 2 {
        if cleaned[i] != cleaned[n - 1 - i] {
            return false;
        }
    }
    true
}

fn caesar(s: &str, shift: u8) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                (((c as u8 - b'a' + shift) % 26) + b'a') as char
            } else if c.is_ascii_uppercase() {
                (((c as u8 - b'A' + shift) % 26) + b'A') as char
            } else {
                c
            }
        })
        .collect()
}

fn word_lengths(s: &str) -> Vec<usize> {
    s.split_whitespace().map(|w| w.len()).collect()
}

fn main() {
    let text = "The quick brown fox jumps over the lazy dog";
    println!("words: {}", text.split_whitespace().count());
    println!("lengths: {:?}", word_lengths(text));
    println!("upper: {}", text.to_uppercase());
    println!("reversed: {}", text.chars().rev().collect::<String>());
    println!("caesar: {}", caesar(text, 3));
    println!("roundtrip ok: {}", caesar(&caesar(text, 3), 23) == text);
    for p in ["A man, a plan, a canal: Panama", "hello", "racecar"] {
        println!("{:?} palindrome: {}", p, is_palindrome(p));
    }
    let vowels = text.chars().filter(|c| "aeiouAEIOU".contains(*c)).count();
    println!("vowels: {}", vowels);
    let mut s = String::new();
    for (i, w) in text.split(' ').enumerate() {
        if i > 0 {
            s.push('-');
        }
        s.push_str(w);
    }
    println!("{}", s);
}
