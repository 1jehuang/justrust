//! Receiver hygiene uses original occurrences, never a printable self fallback.
#[path = "support/source_macro_binding_oracle.rs"]
mod oracle;

#[test]
fn definition_invocation_nested_and_shadowed_receiver_bindings_match_stock() {
    for source in [
        "struct S{v:i32}impl S{fn get(&self)->i32{macro_rules! get{()=>{self.v};}get!()} fn bump(&mut self){macro_rules! bump{()=>{{self.v=self.v+1;}};}bump!();}}fn main(){let mut s=S{v:41};s.bump();assert_eq!(s.get(),42);}",
        "macro_rules! get{($r:ident)=>{$r.v};}struct S{v:i32}impl S{fn get(&self)->i32{get!(self)}}fn main(){let s=S{v:42};assert_eq!(s.get(),42);}",
        "struct S{v:i32}impl S{fn get(&self)->i32{let offset=2;macro_rules! inner{($r:ident)=>{self.v+$r.v+offset};}macro_rules! outer{($r:ident)=>{inner!($r)};} {let offset=100;outer!(self)}}}fn main(){let s=S{v:20};assert_eq!(s.get(),42);}",
        "macro_rules! get{($x:ident)=>{$x};}fn main(){let x=42;assert_eq!(get!(x),42);}",
        "mod m{macro_rules! get{($r:ident)=>{$r.v};}struct S{v:i32}impl S{fn get(&self)->i32{get!(self)}}pub fn run()->i32{let s=S{v:42};s.get()}}fn main(){assert_eq!(m::run(),42);}",
        "fn base()->i32{2}struct S{v:i32}impl S{fn get(self)->i32{macro_rules! value{()=>{self.v+self::base()};}value!()}}fn main(){let s=S{v:40};assert_eq!(s.get(),42);}",
    ] {
        oracle::stock_native(source);
    }
}
