use std::collections::HashMap;
#[derive(Debug, Clone)] enum Ins { Push(i64), Load(String), Store(String), Add, Sub, Mul, Lt, Jz(usize), Jmp(usize), Print, Call(usize), Ret, Halt }
struct Vm { stack: Vec<i64>, vars: HashMap<String, i64>, calls: Vec<usize>, out: Vec<String>, steps: u64 }
impl Vm {
    fn run(&mut self, prog: &[Ins]) -> Result<(), String> {
        let mut pc = 0;
        loop {
            self.steps += 1; if self.steps > 100_000 { return Err("step limit".into()); }
            let ins = prog.get(pc).ok_or("pc out of range")?.clone(); pc += 1;
            match ins {
                Ins::Push(n) => self.stack.push(n), Ins::Load(v) => self.stack.push(*self.vars.get(&v).ok_or(format!("undefined {v}"))?),
                Ins::Store(v) => { let x = self.pop()?; self.vars.insert(v, x); }
                Ins::Add | Ins::Sub | Ins::Mul | Ins::Lt => { let b = self.pop()?; let a = self.pop()?; self.stack.push(match ins { Ins::Add => a.wrapping_add(b), Ins::Sub => a - b, Ins::Mul => a.wrapping_mul(b), _ => (a < b) as i64 }); }
                Ins::Jz(t) => if self.pop()? == 0 { pc = t }, Ins::Jmp(t) => pc = t,
                Ins::Print => { let v = self.pop()?; self.out.push(v.to_string()); }
                Ins::Call(t) => { self.calls.push(pc); pc = t; } Ins::Ret => pc = self.calls.pop().ok_or("ret underflow")?, Ins::Halt => return Ok(()),
            }
        }
    }
    fn pop(&mut self) -> Result<i64, String> { self.stack.pop().ok_or_else(|| "stack underflow".to_string()) }
}
fn assemble(src: &str) -> Vec<Ins> {
    src.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| { let mut p = l.split_whitespace(); let op = p.next().unwrap(); let arg = p.next(); let n = || arg.unwrap().parse::<usize>().unwrap();
        match op { "push" => Ins::Push(arg.unwrap().parse().unwrap()), "load" => Ins::Load(arg.unwrap().into()), "store" => Ins::Store(arg.unwrap().into()), "add" => Ins::Add, "sub" => Ins::Sub, "mul" => Ins::Mul, "lt" => Ins::Lt, "jz" => Ins::Jz(n()), "jmp" => Ins::Jmp(n()), "print" => Ins::Print, "call" => Ins::Call(n()), "ret" => Ins::Ret, _ => Ins::Halt } }).collect()
}
fn main() {
    let fact = "push 1\nstore acc\npush 1\nstore i\nload i\npush 21\nlt\njz 18\nload acc\nload i\nmul\nstore acc\nload i\npush 1\nadd\nstore i\njmp 4\nhalt\nload acc\nprint\ncall 22\nhalt\npush 7\nprint\nret";
    let mut vm = Vm { stack: vec![], vars: HashMap::new(), calls: vec![], out: vec![], steps: 0 };
    let p = assemble(fact); let r = vm.run(&p); println!("{:?} out={:?} steps={}", r, vm.out, vm.steps);
    let mut vm2 = Vm { stack: vec![], vars: HashMap::new(), calls: vec![], out: vec![], steps: 0 };
    println!("{:?}", vm2.run(&assemble("add")));
    println!("{:?}", vm2.run(&assemble("jmp 0")));
}
