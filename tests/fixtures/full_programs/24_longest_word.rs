fn main(){let words="the quick brown fox jumps over the lazy dog";let mut longest="";for w in words.split(' '){if w.len()>longest.len(){longest=w;}}println!("{} {}",longest,words.len());}
