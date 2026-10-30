#![deny(elided_lifetimes_in_paths)]
#![allow(dead_code, unused)]
pub struct Foo<'a>(pub &'a ());
pub struct Two<'a, T>(pub &'a T);
pub enum E<'a> { A(&'a ()) }
pub trait Tr<'a> { fn get(&self) -> &'a (); }
pub type Alias<'a> = Foo<'a>;
pub trait T4 { fn n() -> u8; }
impl<'a> T4 for Foo<'a> { fn n() -> u8 { 1 } }
pub fn f() -> u8 { <Foo as T4>::n() }
