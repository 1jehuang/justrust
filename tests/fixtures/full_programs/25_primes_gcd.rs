fn gcd(a:u32,b:u32)->u32{if b==0{a}else{gcd(b,a%b)}}
fn main(){let mut primes=Vec::new();let mut n=2u32;while primes.len()<10{if primes.iter().all(|p|n%p!=0){primes.push(n);}n+=1;}println!("{:?} {}",primes,gcd(84,36));}
