struct Account {
    owner: String,
    balance: i64,
    history: Vec<i64>,
}

impl Account {
    fn new(owner: &str) -> Self {
        Account { owner: owner.to_string(), balance: 0, history: Vec::new() }
    }

    fn deposit(&mut self, amount: i64) {
        self.balance += amount;
        self.history.push(amount);
    }

    fn withdraw(&mut self, amount: i64) -> bool {
        if amount > self.balance {
            return false;
        }
        self.balance -= amount;
        self.history.push(-amount);
        true
    }

    fn balance(&self) -> i64 {
        self.balance
    }
}

fn transfer(from: &mut Account, to: &mut Account, amount: i64) -> bool {
    if from.withdraw(amount) {
        to.deposit(amount);
        true
    } else {
        false
    }
}

fn main() {
    let mut alice = Account::new("alice");
    let mut bob = Account::new("bob");
    alice.deposit(100);
    bob.deposit(20);
    println!("transfer 30: {}", transfer(&mut alice, &mut bob, 30));
    println!("transfer 500: {}", transfer(&mut alice, &mut bob, 500));
    println!("{}: {}", alice.owner, alice.balance());
    println!("{}: {}", bob.owner, bob.balance());
    println!("alice history: {:?}", alice.history);
    println!("bob transactions: {}", bob.history.len());
}
