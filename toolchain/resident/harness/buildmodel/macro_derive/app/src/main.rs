use macros::Named;

trait Named {
    fn name() -> &'static str;
}

#[derive(Named)]
struct Widget;

#[derive(Named)]
enum Choice {
    #[allow(dead_code)]
    One,
}

fn main() {
    println!("{} {}", Widget::name(), Choice::name());
}
