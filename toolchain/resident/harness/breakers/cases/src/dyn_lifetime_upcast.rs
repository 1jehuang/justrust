//! toml_parser passes a `&mut dyn StringBuilder<'i>` on as one of a shorter lifetime. The
//! two traits differ in a lifetime only, and the coercion was taken for an upcast between
//! unrelated traits.

trait Sink<'i> {
    fn push(&mut self, part: &'i str) -> bool;
}

impl<'i> Sink<'i> for Vec<&'i str> {
    fn push(&mut self, part: &'i str) -> bool {
        Vec::push(self, part);
        true
    }
}

fn inner<'i>(text: &'i str, out: &mut dyn Sink<'i>) -> bool {
    out.push(text)
}

fn outer<'i>(text: &'i str, out: &mut dyn Sink<'i>) -> bool {
    let (head, tail) = text.split_at(1);
    inner(head, out) && inner(tail, out)
}

struct Raw<'i>(&'i str);

impl<'i> Raw<'i> {
    fn decode(&self, out: &mut dyn Sink<'i>) -> bool {
        outer(self.0, out)
    }
}

pub fn run() -> (bool, Vec<&'static str>) {
    let mut parts = Vec::new();
    (Raw("key").decode(&mut parts), parts)
}
