trait Foo {
    type Context<'c>
    where
        Self: 'c;
}
//@ ignore-parallel-frontend query cycle
impl Foo for Box<dyn Foo> {}
//~^ ERROR cycle detected

fn main() {}
