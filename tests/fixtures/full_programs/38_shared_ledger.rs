use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug)]
struct Account {
    name: String,
    balance: i64,
    entries: Vec<String>,
}

struct Ledger {
    accounts: Vec<Rc<RefCell<Account>>>,
    committed: usize,
}

impl Ledger {
    fn open(&mut self, name: &str, balance: i64) -> Rc<RefCell<Account>> {
        let account = Rc::new(RefCell::new(Account {
            name: name.to_string(),
            balance,
            entries: Vec::new(),
        }));
        self.accounts.push(Rc::clone(&account));
        account
    }

    fn transfer(
        &mut self,
        from: &Rc<RefCell<Account>>,
        to: &Rc<RefCell<Account>>,
        amount: i64,
    ) -> Result<(), String> {
        if amount <= 0 {
            return Err("amount must be positive".to_string());
        }
        if Rc::ptr_eq(from, to) {
            return Err("same account".to_string());
        }
        let mut debit = match from.try_borrow_mut() {
            Ok(guard) => guard,
            Err(error) => return Err(format!("source busy: {}", error)),
        };
        let mut credit = match to.try_borrow_mut() {
            Ok(guard) => guard,
            Err(error) => return Err(format!("destination busy: {}", error)),
        };
        if debit.balance < amount {
            return Err(format!("insufficient funds: {}", debit.name));
        }
        debit.balance -= amount;
        credit.balance += amount;
        let debit_entry = format!("to {}: {}", credit.name, amount);
        let credit_entry = format!("from {}: {}", debit.name, amount);
        debit.entries.push(debit_entry);
        credit.entries.push(credit_entry);
        self.committed += 1;
        Ok(())
    }

    fn report(&self) {
        let mut total = 0i64;
        for account in &self.accounts {
            let view = account.try_borrow().unwrap();
            total += view.balance;
            println!("{}: {} ({} entries)", view.name, view.balance, view.entries.len());
            for entry in &view.entries {
                println!("  {}", entry);
            }
        }
        println!("total {} across {} transfers", total, self.committed);
    }
}

fn main() {
    let mut ledger = Ledger { accounts: Vec::new(), committed: 0 };
    let alice = ledger.open("Alice", 1000);
    let bob = ledger.open("Bob", 300);
    let reserve = ledger.open("Reserve", 200);
    let teller = Rc::clone(&alice);
    ledger.transfer(&teller, &bob, 125).unwrap();
    ledger.transfer(&bob, &reserve, 50).unwrap();
    let statement = bob.borrow();
    println!("statement Bob: {}", statement.balance);
    println!("blocked: {}", ledger.transfer(&alice, &bob, 10).unwrap_err());
    drop(statement);
    // A failed destination acquisition must release the source guard.
    ledger.transfer(&alice, &bob, 10).unwrap();
    println!("rejected: {}", ledger.transfer(&bob, &alice, 1000).unwrap_err());
    println!("rejected: {}", ledger.transfer(&alice, &teller, 1).unwrap_err());
    {
        let edit = reserve.borrow_mut();
        println!("read blocked: {}", reserve.try_borrow().unwrap_err());
        println!("reserve held: {}", edit.balance);
    }
    println!("aliases {} same {}", Rc::strong_count(&alice), Rc::ptr_eq(&alice, &teller));
    ledger.report();
}
