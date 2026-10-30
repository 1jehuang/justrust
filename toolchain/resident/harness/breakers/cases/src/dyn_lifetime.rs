//! toml_parser passes a `&mut dyn StringBuilder<'i>` on as a `&mut dyn StringBuilder<'_>`:
//! a coercion between two trait objects that only a lifetime of the trait tells apart, and
//! no upcast.

trait Builder<'s> {
    fn push(&mut self, text: &'s str) -> bool;
}

impl<'s> Builder<'s> for Vec<&'s str> {
    fn push(&mut self, text: &'s str) -> bool {
        Vec::push(self, text);
        true
    }
}

fn halves<'i>(text: &'i str, out: &mut dyn Builder<'i>) -> bool {
    let (left, right) = text.split_at(text.len() / 2);
    whole(left, out) && whole(right, out)
}

fn whole<'i>(text: &'i str, out: &mut dyn Builder<'i>) -> bool {
    out.push(text)
}

pub fn run() -> Vec<String> {
    let text = String::from("leftright");
    let mut parts: Vec<&str> = Vec::new();
    halves(&text, &mut parts);
    parts.into_iter().map(str::to_owned).collect()
}
