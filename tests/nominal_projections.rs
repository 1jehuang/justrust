//! Declaration-owned projections in struct and enum field schemas.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn nested_struct_projection_executes() {
    oracle::stock_native(
        "trait It{type Item;}struct C;impl It for C{type Item=i32;}struct Peek<I:It>{item:Option<Option<I::Item>>}fn main(){let p:Peek<C>=Peek{item:Some(Some(7))};if let Some(Some(n))=p.item{println!(\"{}\",n);}}",
    );
}

#[test]
fn enum_projection_executes() {
    oracle::stock_native(
        "trait It{type Item;}struct C;impl It for C{type Item=i32;}enum Value<I:It>{Item(I::Item),Empty}fn main(){let v:Value<C>=Value::Item(9);if let Value::Item(n)=v{println!(\"{}\",n);}}",
    );
}

#[test]
fn unused_nominal_projection_without_membership_is_rejected() {
    oracle::stock_and_native_reject(
        "trait It{type Item;}struct Bad<I>{item:<I as It>::Item}fn main(){}",
    );
}

#[test]
fn unused_nominal_projection_with_wrong_member_is_rejected() {
    oracle::stock_and_native_reject(
        "trait It{type Item;}struct Bad<I:It>{item:<I as It>::Missing}fn main(){}",
    );
}

#[test]
fn unused_nominal_projection_with_wrong_ordered_membership_is_rejected() {
    oracle::stock_and_native_reject(
        "trait It<A,B>{type Item;}struct Bad<I:It<i32,bool>>{item:<I as It<bool,i32>>::Item}fn main(){}",
    );
}

#[test]
fn generic_native_instances_payload_owners_recursion_and_aggregate_abi() {
    oracle::stock_native_with_args(
        r#"
            struct Wrap<T> { value:T }
            enum Maybe<T> { None, Some(T) }
            fn identity<T>(value:T)->T { value }
            fn forward<T>(value:T)->T { identity::<T>(value) }
            fn packed<T>(value:T)->Wrap<T> { Wrap::<T> { value } }
            fn unpack<T>(value:Wrap<T>)->T { value.value }
            fn recur<T>(value:T, count:i32)->T {
                if count == 0 { value } else { recur::<T>(value,count-1) }
            }
            fn even<T>(value:T, count:i32)->T {
                if count == 0 { value } else { odd::<T>(value,count-1) }
            }
            fn odd<T>(value:T, count:i32)->T {
                if count == 0 { value } else { even::<T>(value,count-1) }
            }
            fn main() {
                if identity::<i8>(-127i8) != -127i8 { 1/0; }
                if identity::<u64>(18446744073709551615u64) != 18446744073709551615u64 { 1/0; }
                let narrow=packed::<u8>(255u8);
                if unpack::<u8>(narrow) != 255u8 { 1/0; }
                let wide=packed::<u64>(18446744073709551615u64);
                let selected=Maybe::<Wrap<u64>>::Some(wide);
                let number=match selected {
                    Maybe::<Wrap<u64>>::None => 0u64,
                    Maybe::<Wrap<u64>>::Some(inner) => unpack::<u64>(inner),
                };
                if number != 18446744073709551615u64 { 1/0; }
                let tuple=forward::<(u64,bool)>((18446744073709551615u64,true));
                if tuple.0 != 18446744073709551615u64 { 1/0; }
                if !tuple.1 { 1/0; }
                let array=unpack::<[u64;9]>(packed::<[u64;9]>([18446744073709551615u64;9]));
                let array=recur::<[u64;9]>(array,3);
                let array=even::<[u64;9]>(array,4);
                if array[0] != 18446744073709551615u64 { 1/0; }
                if array[8] != 18446744073709551615u64 { 1/0; }
                println!("generic-native");
            }
            "#,
        &["-Aunconditional_panic"],
    );
}

#[test]
fn generic_reference_native_trait_refinement_preserves_actual_return_pointers() {
    oracle::stock_native_with_args(
        r#"
            trait View {
                fn field(&self)->&u64;
                fn field_mut(&mut self)->&mut u64;
            }
            trait Pick { fn pick<'a>(known:&'a u64,whole:&'a Self)->&'a Self; }
            mod hidden {
                pub struct Data { pad:[u64;9],value:u64 }
                impl crate::View for Data {
                    fn field<'p>(&'p self)->&'p u64 { &self.value }
                    fn field_mut<'q>(&'q mut self)->&'q mut u64 { &mut self.value }
                }
                impl crate::Pick for Data {
                    fn pick<'a>(known:&'a u64,whole:&'a Self)->&'a Self { whole }
                }
                pub fn make()->Data {
                    Data { pad:[18446744073709551615u64;9],value:18446744073709551615u64 }
                }
            }
            struct Wrapper<T> { inner:T }
            impl<T:View> View for Wrapper<T> {
                fn field(&self)->&u64 { self.inner.field() }
                fn field_mut(&mut self)->&mut u64 { self.inner.field_mut() }
            }
            fn view<T:View>(value:&T)->&u64 { value.field() }
            fn view_mut<T:View>(value:&mut T)->&mut u64 { value.field_mut() }
            fn concrete(value:&hidden::Data)->&u64 { value.field() }
            fn pick<'a,T:Pick>(known:&'a u64,whole:&'a T)->&'a T { T::pick(known,whole) }
            fn choose<'a,'b,T,U>(left:&'a T,right:&'b U)->&'a T { left }
            fn main() {
                let mut wrapped=Wrapper::<hidden::Data> { inner:hidden::make() };
                if *view::<Wrapper<hidden::Data>>(&wrapped) != 18446744073709551615u64 { 1/0; }
                *view_mut::<Wrapper<hidden::Data>>(&mut wrapped)=9223372036854775808u64;
                if *concrete(&wrapped.inner) != 9223372036854775808u64 { 1/0; }
                let known=1u64;
                let selected=pick::<hidden::Data>(&known,&wrapped.inner);
                if *selected.field() != 9223372036854775808u64 { 1/0; }
                let outer=17u64;
                let chosen:&u64;
                { let short=19u64; chosen=choose::<u64,u64>(&outer,&short); }
                if *chosen != 17u64 { 1/0; }
                println!("generic-reference-trait-native");
            }
            "#,
        &["-Aunconditional_panic"],
    );
}

#[test]
fn trait_native_conditional_dispatch_preserves_scalar_aggregate_abi_and_receiver_order() {
    oracle::stock_native_with_args(
        r#"
            trait Pass {
                fn take(value: Self) -> Self;
                fn relay(self) -> Self;
                fn code(self) -> u64;
            }
            enum Tag { Ready }
            impl Pass for Tag {
                fn take(value: Self) -> Self { value }
                fn relay(self) -> Self { self }
                fn code(self) -> u64 { 41u64 }
            }
            struct Packet<T> { value: T, words: [u64;9] }
            impl<T: Pass> Pass for Packet<T> {
                fn take(value: Self) -> Self { value }
                fn relay(self) -> Self { self }
                fn code(self) -> u64 {
                    if self.words[8] != 18446744073709551615u64 { 1/0; }
                    self.value.code()
                }
            }
            trait Combine { fn combine(self, extra: u64) -> u64; }
            impl<T: Pass> Combine for Packet<T> {
                fn combine(mut self, extra: u64) -> u64 {
                    self.words[0] = extra;
                    self.value.code() + self.words[0]
                }
            }
            fn relay<T: Pass>(value: T) -> T { T::take(value.relay()) }
            fn read<T: Pass>(value: T) -> u64 { <T as Pass>::code(value) }
            fn receiver() -> Packet<Tag> {
                println!("receiver");
                Packet::<Tag> { value: Tag::Ready, words: [18446744073709551615u64;9] }
            }
            fn argument() -> u64 { println!("argument"); 1u64 }
            fn main() {
                if read::<Tag>(relay::<Tag>(Tag::Ready)) != 41u64 { 1/0; }
                let packet = Packet::<Tag> {
                    value: Tag::Ready,
                    words: [18446744073709551615u64;9],
                };
                if read::<Packet<Tag>>(relay::<Packet<Tag>>(packet)) != 41u64 { 1/0; }
                if receiver().combine(argument()) != 42u64 { 1/0; }
                println!("trait-native");
            }
            "#,
        &["-Aunconditional_panic"],
    );
}
