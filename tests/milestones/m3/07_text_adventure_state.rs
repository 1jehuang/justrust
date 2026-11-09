use std::collections::HashMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] enum Room { Hall, Library, Vault, Garden }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] enum Dir { N, S, E, W }
#[derive(Debug)] enum Cmd { Go(Dir), Take(String), Use(String), Look, Inv }
fn parse(s: &str) -> Option<Cmd> { let mut w = s.split_whitespace(); Some(match (w.next()?, w.next()) { ("go", Some(d)) => Cmd::Go(match d { "n" => Dir::N, "s" => Dir::S, "e" => Dir::E, "w" => Dir::W, _ => return None }), ("take", Some(x)) => Cmd::Take(x.into()), ("use", Some(x)) => Cmd::Use(x.into()), ("look", None) => Cmd::Look, ("inv", None) => Cmd::Inv, _ => return None }) }
struct Game { at: Room, exits: HashMap<(Room, Dir), Room>, items: HashMap<Room, Vec<String>>, inv: Vec<String>, vault_open: bool }
impl Game {
    fn step(&mut self, c: Cmd) -> String {
        match c {
            Cmd::Go(d) => match self.exits.get(&(self.at, d)).copied() { Some(Room::Vault) if !self.vault_open => "the vault is locked".into(), Some(r) => { self.at = r; format!("you enter {:?}", r) } None => "no way".into() },
            Cmd::Take(x) => { let v = self.items.entry(self.at).or_default(); if let Some(i) = v.iter().position(|y| *y == x) { self.inv.push(v.remove(i)); format!("took {x}") } else { format!("no {x} here") } }
            Cmd::Use(x) if x == "key" && self.inv.contains(&x) && self.at == Room::Library => { self.vault_open = true; "click".into() }
            Cmd::Use(x) => format!("can't use {x}"),
            Cmd::Look => format!("{:?}: {:?}", self.at, self.items.get(&self.at).cloned().unwrap_or_default()),
            Cmd::Inv => format!("{:?}", self.inv),
        }
    }
}
fn main() {
    let exits = HashMap::from([((Room::Hall, Dir::N), Room::Library), ((Room::Library, Dir::S), Room::Hall), ((Room::Library, Dir::E), Room::Vault), ((Room::Vault, Dir::W), Room::Library), ((Room::Hall, Dir::W), Room::Garden), ((Room::Garden, Dir::E), Room::Hall)]);
    let items = HashMap::from([(Room::Garden, vec!["key".to_string(), "rose".to_string()]), (Room::Vault, vec!["gold".to_string()])]);
    let mut g = Game { at: Room::Hall, exits, items, inv: vec![], vault_open: false };
    for line in ["look", "go n", "go e", "go s", "go w", "take key", "take sword", "go e", "go n", "use key", "go e", "take gold", "inv", "dance", "go x"] {
        match parse(line) { Some(c) => println!("> {line}: {}", g.step(c)), None => println!("> {line}: ?") }
    }
}
