fn fib(n:u64)->u64{if n<2{n}else{fib(n-1)+fib(n-2)}}
fn main(){let mut v=Vec::new();for i in 0..10{v.push(fib(i));}let s:u64=v.iter().sum();println!("{:?} {}",v,s);}
