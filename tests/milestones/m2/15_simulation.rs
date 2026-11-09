use std::collections::{HashMap, VecDeque};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] enum Kind { Cpu, Io }
#[derive(Debug)] struct Job { id: u32, kind: Kind, cost: u32 }
struct Rng(u64);
impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } }
trait Scheduler { fn pick(&mut self, q: &mut VecDeque<Job>) -> Option<Job>; fn label(&self) -> &'static str; }
struct Fifo; struct ShortestFirst;
impl Scheduler for Fifo { fn pick(&mut self, q: &mut VecDeque<Job>) -> Option<Job> { q.pop_front() } fn label(&self) -> &'static str { "fifo" } }
impl Scheduler for ShortestFirst { fn pick(&mut self, q: &mut VecDeque<Job>) -> Option<Job> { let i = q.iter().enumerate().min_by_key(|(_, j)| j.cost)?.0; q.remove(i) } fn label(&self) -> &'static str { "sjf" } }
fn run(s: &mut dyn Scheduler, seed: u64) -> (u64, HashMap<Kind, u32>) {
    let mut rng = Rng(seed); let mut q: VecDeque<Job> = (0..12).map(|id| { let r = rng.next(); Job { id, kind: if r % 3 == 0 { Kind::Io } else { Kind::Cpu }, cost: (r % 17 + 1) as u32 } }).collect();
    let (mut t, mut wait) = (0u64, 0u64); let mut per: HashMap<Kind, u32> = HashMap::new();
    while let Some(j) = s.pick(&mut q) { wait += t; t += j.cost as u64; *per.entry(j.kind).or_default() += j.cost; if j.id == 0 { println!("  job0 {:?}", j); } }
    (wait, per)
}
fn main() {
    let mut scheds: Vec<Box<dyn Scheduler>> = vec![Box::new(Fifo), Box::new(ShortestFirst)];
    for s in scheds.iter_mut() { let (w, per) = run(s.as_mut(), 0x2545F4914F6CDD1D); let mut per: Vec<_> = per.into_iter().collect(); per.sort_by_key(|(k, _)| *k as u8); println!("{} wait={} {:?}", s.label(), w, per); }
}
