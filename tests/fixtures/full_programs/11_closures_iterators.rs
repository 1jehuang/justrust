fn apply<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 {
    f(x)
}

fn make_adder(n: i32) -> impl Fn(i32) -> i32 {
    move |x| x + n
}

fn compose<A, B, C>(f: impl Fn(A) -> B, g: impl Fn(B) -> C) -> impl Fn(A) -> C {
    move |x| g(f(x))
}

fn main() {
    let nums: Vec<i32> = (1..=10).collect();
    let evens: Vec<i32> = nums.iter().filter(|&&x| x % 2 == 0).cloned().collect();
    let squares: Vec<i32> = nums.iter().map(|x| x * x).collect();
    let sum: i32 = nums.iter().sum();
    let product: i64 = nums.iter().map(|&x| x as i64).product();
    println!("evens: {:?}", evens);
    println!("squares: {:?}", squares);
    println!("sum = {}, product = {}", sum, product);
    let add5 = make_adder(5);
    println!("apply add5 to 10: {}", apply(&add5, 10));
    let double_then_add = compose(|x: i32| x * 2, make_adder(1));
    println!("compose(3) = {}", double_then_add(3));
    let mut counter = 0;
    let mut inc = || {
        counter += 1;
        counter
    };
    inc();
    inc();
    println!("counter = {}", inc());
    let max = nums.iter().copied().fold(i32::MIN, i32::max);
    println!("max = {}", max);
    let pos = nums.iter().position(|&x| x == 7);
    println!("position of 7: {:?}", pos);
    let any_big = nums.iter().any(|&x| x > 9);
    let all_pos = nums.iter().all(|&x| x > 0);
    println!("any > 9: {}, all > 0: {}", any_big, all_pos);
    let pairs: Vec<(i32, char)> = nums.iter().copied().zip("abc".chars()).collect();
    println!("pairs: {:?}", pairs);
    let chunked: Vec<i32> = nums.chunks(3).map(|c| c.iter().sum()).collect();
    println!("chunk sums: {:?}", chunked);
    let taken: Vec<i32> = nums.iter().rev().skip(2).step_by(3).take(3).copied().collect();
    println!("taken: {:?}", taken);
}
