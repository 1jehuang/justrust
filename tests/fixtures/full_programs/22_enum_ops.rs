#[derive(Debug,Clone,Copy,PartialEq)]enum Op{Add,Mul}
fn eval(op:Op,a:i64,b:i64)->i64{match op{Op::Add=>a+b,Op::Mul=>a*b}}
fn main(){let ops=[Op::Add,Op::Mul,Op::Add];let mut acc=1i64;for (i,op) in ops.iter().enumerate(){acc=eval(*op,acc,i as i64+2);}println!("{} {:?}",acc,ops[1]);}
