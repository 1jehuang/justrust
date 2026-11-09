use std::collections::HashMap;
use std::fmt;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)] struct AccountId(u32);
#[derive(Debug)] enum TxError { Insufficient { need: u64, have: u64 }, NoAccount(AccountId), Frozen(AccountId) }
impl fmt::Display for TxError { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { match self { TxError::Insufficient { need, have } => write!(f, "need {need} have {have}"), TxError::NoAccount(a) => write!(f, "no account {}", a.0), TxError::Frozen(a) => write!(f, "frozen {}", a.0) } } }
trait Ledger { fn balance(&self, id: AccountId) -> Result<u64, TxError>; fn transfer(&mut self, from: AccountId, to: AccountId, amt: u64) -> Result<(), TxError>; }
#[derive(Default)] struct Bank { accts: HashMap<AccountId, (u64, bool)>, log: Vec<String> }
impl Ledger for Bank {
    fn balance(&self, id: AccountId) -> Result<u64, TxError> { self.accts.get(&id).map(|a| a.0).ok_or(TxError::NoAccount(id)) }
    fn transfer(&mut self, from: AccountId, to: AccountId, amt: u64) -> Result<(), TxError> {
        let have = self.balance(from)?; self.balance(to)?;
        if self.accts[&from].1 { return Err(TxError::Frozen(from)); }
        let rest = have.checked_sub(amt).ok_or(TxError::Insufficient { need: amt, have })?;
        self.accts.get_mut(&from).unwrap().0 = rest; self.accts.get_mut(&to).unwrap().0 += amt;
        self.log.push(format!("{}->{}:{}", from.0, to.0, amt)); Ok(())
    }
}
fn batch<L: Ledger>(l: &mut L, ops: &[(u32, u32, u64)]) -> Vec<Result<(), TxError>> { ops.iter().map(|&(a, b, x)| l.transfer(AccountId(a), AccountId(b), x)).collect() }
fn main() {
    let mut b = Bank::default();
    for (i, bal) in [(1, 100), (2, 50), (3, 0)] { b.accts.insert(AccountId(i), (bal, i == 3)); }
    for r in batch(&mut b, &[(1, 2, 30), (2, 3, 100), (3, 1, 1), (9, 1, 5), (2, 1, 80)]) { match r { Ok(()) => println!("ok"), Err(e) => println!("err: {e}") } }
    let mut ids: Vec<_> = b.accts.keys().copied().collect(); ids.sort();
    for id in ids { println!("{:?} = {}", id, b.balance(id).unwrap()); }
    println!("{}", b.log.join(" "));
}
