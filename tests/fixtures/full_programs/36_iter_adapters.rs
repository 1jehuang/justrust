fn main() {
    let v: Vec<i32> = (1..=10).collect();
    let evens: Vec<i32> = v.iter().copied().filter(|x| x % 2 == 0).collect();
    println!("{:?}", evens);
    let pairs: Vec<(usize, &i32)> = v.iter().enumerate().take(3).collect();
    println!("{:?}", pairs);
    let zipped: Vec<(i32, char)> = v.iter().cloned().zip("abc".chars()).collect();
    println!("{:?}", zipped);
    println!("{}", v.iter().any(|&x| x > 9));
    println!("{}", v.iter().all(|&x| x > 0));
    println!("{:?}", v.iter().position(|&x| x == 4));
    let s: i32 = v.iter().skip(2).step_by(3).sum();
    println!("{}", s);
    let folded = v.iter().fold(String::new(), |acc, x| acc + &x.to_string());
    println!("{}", folded);
    println!("{:?}", v.chunks(4).map(|c| c.len()).collect::<Vec<_>>());
    println!("{:?}", v.windows(2).map(|w| w[1] - w[0]).max());
}
