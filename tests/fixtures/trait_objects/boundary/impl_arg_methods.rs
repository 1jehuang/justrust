trait Score {
    fn score(&self) -> u32;
}
struct Player {
    points: u32,
}
impl Score for Player {
    fn score(&self) -> u32 {
        self.points
    }
}
struct Board {
    bonus: u32,
}
impl Board {
    fn total(&self, who: &impl Score) -> u32 {
        who.score() + self.bonus
    }
}
fn main() {
    let board = Board { bonus: 5 };
    let p = Player { points: 7 };
    if board.total(&p) == 12 {
        println!("inherent method impl arg");
    }
}
