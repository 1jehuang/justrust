//! Stock-first normal Drop integration. No rejection of a Rust-valid unsupported
//! program counts as language conformance. Run this same target in bootstrap
//! debug and release profiles; the native compiler never delegates to rustc.
#![allow(clippy::duplicate_mod)]
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use stock_rustc::stock_rustc;

use justrust_engine::compiler;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "support/closure_oracle.rs"]
mod explicit_drop_oracle;

#[test]
fn explicit_drop_function_matches_stock() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check(
        "explicit-drop",
        &[
            Case {
                name: "prelude_core_std_and_reference_drop",
                source: r#"struct D{id:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.id);}}
fn main(){let first=D{id:1};println!("before");drop(first);println!("after");core::mem::drop((D{id:2},D{id:3}));std::mem::drop(String::from("owned"));let mut n: i32=4;drop(&mut n);n+=1;println!("{}",n);let captured=D{id:4};let f=move||{let _=&captured;};drop(f);println!("done");}"#,
                expect: Expect::Run("before\ndrop 1\nafter\ndrop 2\ndrop 3\n5\ndrop 4\ndone\n"),
            },
            Case {
                name: "move_after_explicit_drop",
                source: "struct D;fn main(){let value=D;drop(value);let again=value;}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "drop_cannot_invalidate_live_borrow",
                source: "struct D{id:i32}fn main(){let value=D{id:1};let r=&value;drop(value);println!(\"{}\",r.id);}",
                expect: Expect::Invalid("E0505"),
            },
            Case {
                name: "borrowed_destructor_temporaries_drop_at_statement_and_condition_end",
                source: r#"use std::cell::RefCell;
struct D(u32);impl Drop for D{fn drop(&mut self){println!("drop {}",self.0);}}impl D{fn v(&self)->u32{self.0}}
fn mk(n:u32)->D{D(n)}fn show(d:&D)->u32{d.0}fn bx()->Box<u32>{Box::new(9)}
fn main(){let a=show(&mk(1))+show(&mk(2));println!("a {}",a);if show(&mk(4))>3{println!("in");}else if mk(7).v()>1{println!("no");}
match show(&mk(5)){5=>println!("five"),_=>{}}let c=RefCell::new(5i32);*c.borrow_mut()+=1;let v=*c.borrow();println!("{} {}",v,*bx());}"#,
                expect: Expect::Run("drop 2\ndrop 1\na 3\ndrop 4\nin\nfive\ndrop 5\n6 9\n"),
            },
        ],
    );
}

#[test]
fn direct_let_borrowed_destructor_temporaries_match_stock() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check(
        "direct-let-borrowed-destructor-temporaries",
        &[
            Case {
                name: "lexical_owners_outlive_last_use_and_nested_blocks_drop_in_reverse_order",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.n);}}
fn main(){let first=D{n:1};let borrowed=&D{n:2};println!("value {}",borrowed.n);
{let nested=&D{n:3};let last=D{n:4};println!("nested {}",nested.n);}
println!("outer");let last=D{n:5};println!("end");}"#,
                expect: Expect::Run(
                    "value 2\nnested 3\ndrop 4\ndrop 3\nouter\nend\ndrop 5\ndrop 2\ndrop 1\n",
                ),
            },
            Case {
                name: "mutable_borrow_owns_the_original_storage_until_block_exit",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.n);}}
fn main(){let borrowed=&mut D{n:6};borrowed.n=7;let shared=&*borrowed;
println!("value {}",shared.n);println!("end");}"#,
                expect: Expect::Run("value 7\nend\ndrop 7\n"),
            },
            Case {
                name: "factory_runs_once_and_shadowed_reference_does_not_end_first_owner",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.n);}}
fn make(n:i32)->D{println!("make {}",n);D{n}}
fn main(){let borrowed=&make(1);println!("first {}",borrowed.n);
let borrowed=&make(2);println!("second {}",borrowed.n);println!("end");}"#,
                expect: Expect::Run("make 1\nfirst 1\nmake 2\nsecond 2\nend\ndrop 2\ndrop 1\n"),
            },
            Case {
                name: "expression_statement_is_not_an_extending_let",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.n);}}
fn main(){&D{n:1};println!("statement ended");let borrowed=&D{n:2};
println!("value {}",borrowed.n);&D{n:3};println!("end");}"#,
                expect: Expect::Run("drop 1\nstatement ended\nvalue 2\ndrop 3\nend\ndrop 2\n"),
            },
            Case {
                name: "normal_early_return_cleans_initialized_extended_owners",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.n);}}
fn run(early:bool){let before=D{n:1};let borrowed=&D{n:2};let after=D{n:3};
println!("value {}",borrowed.n);if early{return;}println!("tail");}
fn main(){run(true);println!("between");run(false);println!("end");}"#,
                expect: Expect::Run(
                    "value 2\ndrop 3\ndrop 2\ndrop 1\nbetween\nvalue 2\ntail\ndrop 3\ndrop 2\ndrop 1\nend\n",
                ),
            },
            Case {
                name: "return_before_owner_initialization_does_not_destroy_uninitialized_storage",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.n);}}
fn run(){let before=D{n:1};let borrowed=&D{n:{return;}};println!("unreachable");}
fn main(){run();println!("end");}"#,
                expect: Expect::Run("drop 1\nend\n"),
            },
            Case {
                name: "unused_call_argument_does_not_extend_a_temporary",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){}}
fn identity(value:&D)->&D{value}
fn unused(){let borrowed=identity(&D{n:7});println!("{}",borrowed.n);}fn main(){}"#,
                expect: Expect::InvalidAfterParse("E0716"),
            },
            Case {
                name: "unused_outer_reference_cannot_outlive_inner_extended_owner",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){}}
fn unused(){let borrowed:&D;{let inner=&D{n:7};borrowed=inner;}println!("{}",borrowed.n);}fn main(){}"#,
                expect: Expect::InvalidAfterParse("E0716"),
            },
            Case {
                name: "unused_named_owner_still_ends_at_its_own_block_boundary",
                source: r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){}}
fn unused(){let borrowed:&D;{let inner=D{n:7};borrowed=&inner;}println!("{}",borrowed.n);}fn main(){}"#,
                expect: Expect::InvalidAfterParse("E0597"),
            },
        ],
    );
}

#[test]
fn direct_let_borrowed_destructor_panic_cleanup_matches_stock() {
    // Exercise only the existing native abort and bounded unwind protocols.
    // This does not add catch_unwind, foreign unwinding or generic dropck.
    let source = r#"struct D{n:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.n);}}
fn main(){let before=D{n:1};let borrowed=&D{n:2};let after=D{n:3};
println!("value {}",borrowed.n);panic!("borrowed owner panic");}"#;
    let scratch = Scratch::new();
    let input = scratch.write("borrowed-panic.rs", source.as_bytes());
    for unwind in [false, true] {
        let stock_image = scratch.path(&format!("stock-panic-{unwind}"));
        let compiled = capture(
            Command::new(stock_rustc())
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "-Awarnings",
                    if unwind {
                        "-Cpanic=unwind"
                    } else {
                        "-Cpanic=abort"
                    },
                ])
                .arg(&input)
                .arg("-o")
                .arg(&stock_image),
            &scratch,
            30,
        );
        successful(&compiled, "stock borrowed-owner panic compile");
        let stock = capture(
            Command::new(&stock_image).env("RUST_BACKTRACE", "0"),
            &scratch,
            10,
        );
        assert_eq!(
            stock.stdout,
            if unwind {
                b"value 2\ndrop 3\ndrop 2\ndrop 1\n".as_slice()
            } else {
                b"value 2\n".as_slice()
            }
        );
        assert_eq!(stock.status.code(), unwind.then_some(101));
        assert_eq!(stock.status.signal(), (!unwind).then_some(6));
        for workers in [1, 8] {
            for check_only in [true, false] {
                let native = compiler::compile_observed_with_options(
                    source.into(),
                    workers,
                    check_only,
                    compiler::source::SourceOptions {
                        panic_unwind: unwind,
                        ..Default::default()
                    },
                )
                .unwrap_or_else(|failure| {
                    panic!("unwind={unwind} workers={workers} CHECK={check_only}: {failure}")
                });
                assert!(native.analyses.iter().all(|analysis| analysis.passed()));
                assert!(native.report.capture_complete);
                assert_eq!(native.report.events_dropped, 0);
                assert!(
                    native
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(_, count)| *count == 1)
                );
                assert_eq!(native.executable.is_none(), check_only);
                if check_only {
                    assert!(
                        native
                            .report
                            .node_evaluations
                            .iter()
                            .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130 | 131 | 133))
                    );
                    continue;
                }
                let image = scratch.path(&format!("native-panic-{unwind}-{workers}"));
                fs::write(&image, native.executable.unwrap()).unwrap();
                fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
                let ran = capture(Command::new(image).env("RUST_BACKTRACE", "0"), &scratch, 10);
                assert_eq!(ran.status.code(), stock.status.code(), "{ran}");
                assert_eq!(ran.status.signal(), stock.status.signal(), "{ran}");
                assert_eq!(ran.stdout, stock.stdout, "{ran}");
                assert!(
                    String::from_utf8_lossy(&ran.stderr).contains("borrowed owner panic"),
                    "{ran}"
                );
            }
        }
    }
}

#[test]
fn direct_let_borrowed_destructor_extension_keeps_explicit_boundaries() {
    for (index, source) in [
        "struct D{n:i32}impl Drop for D{fn drop(&mut self){}}fn unused(){let references=(&D{n:1},);}fn main(){}",
        "struct D{n:i32}impl Drop for D{fn drop(&mut self){}}fn value(d:&D)->i32{d.n}fn unused(){let reference=&D{n:value(&D{n:1})};}fn main(){}",
    ].into_iter().enumerate() {
        let scratch = Scratch::new();
        let input = scratch.write("boundary.rs", source.as_bytes());
        let stock_image = scratch.path("stock-boundary");
        let stock = capture(
            Command::new(stock_rustc()).env_remove("RUSTC_BOOTSTRAP")
                .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
                .arg(input).arg("-o").arg(stock_image), &scratch, 30,
        );
        successful(&stock, "stock accepts the deliberately unsupported extension boundary");
        for workers in [1, 8] {
            for check_only in [true, false] {
                let failure = match compiler::compile_observed(source.into(), workers, check_only) {
                    Err(failure) => failure,
                    Ok(_) => panic!("boundary={index} workers={workers} CHECK={check_only}: unsupported extension was admitted"),
                };
                assert!(failure.error.to_string().contains("unsupported: destructor temporary lifetime extension requires a direct let borrow without dependent borrowed temporaries"), "boundary={index} workers={workers} CHECK={check_only}: {failure}");
                assert!(failure.resource_usage.first_failure.is_none());
                assert!(!failure.resource_usage.poisoned);
            }
        }
    }
}

#[test]
fn phantom_data_is_copy_without_constructing_or_dropping_its_argument() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check_with_compiler(
        "phantom-data-drop",
        &[
            Case {
                name: "marker_is_nonowning_even_for_destructor_argument",
                source: r#"use core::marker::PhantomData;
struct D{id:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.id);}}
struct Holder<T>{marker:PhantomData<T>,value:i32}
fn consume<T>(marker:PhantomData<T>){drop(marker);}
fn main(){let marker:PhantomData<D> = PhantomData;consume(marker);consume(marker);
let holder=Holder::<D>{marker:PhantomData,value:7};let copied=holder.marker;drop(copied);
println!("value {}",holder.value);drop(holder);println!("marker done");
let real=D{id:9};drop(real);}"#,
                expect: Expect::Run("value 7\nmarker done\ndrop 9\n"),
            },
            Case {
                name: "marker_actuals_remain_distinct",
                source: "use core::marker::PhantomData;fn take(value:PhantomData<i32>){}fn main(){let value:PhantomData<bool> = PhantomData;take(value);}",
                expect: Expect::Invalid("E0308"),
            },
        ],
        Path::new(&stock_rustc()),
    );
}

#[test]
fn manually_drop_preserves_payload_moves_borrows_and_explicit_cleanup() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check_with_compiler(
        "manually-drop-payload",
        &[
            Case {
                name: "stored_payload_suppresses_only_implicit_cleanup",
                source: r#"use core::mem::ManuallyDrop;
struct D{id:i32}impl Drop for D{fn drop(&mut self){println!("drop {}",self.id);}}
fn wrap<T>(value:T)->ManuallyDrop<T>{ManuallyDrop::new(value)}
fn main(){{let _hidden=wrap(D{id:1});}println!("hidden");
let owned=ManuallyDrop::into_inner(wrap(D{id:2}));println!("unpacked");drop(owned);
let mut explicit=ManuallyDrop::new(D{id:3});unsafe{ManuallyDrop::drop(&mut explicit);}println!("explicit");
let mut number=ManuallyDrop::new(4i32);let read=&*number;println!("{}",*read);let write=&mut *number;*write=7;println!("{}",ManuallyDrop::into_inner(number));}"#,
                expect: Expect::Run("hidden\nunpacked\ndrop 2\ndrop 3\nexplicit\n4\n7\n"),
            },
            Case {
                name: "noncopy_payload_deref_cannot_move",
                source: "use core::mem::ManuallyDrop;struct Token;fn main(){let x=ManuallyDrop::new(Token);let a=*x;}",
                expect: Expect::Invalid("E0507"),
            },
            Case {
                name: "noncopy_payload_borrows_and_copy_fields_still_work",
                source: r#"use core::mem::ManuallyDrop;struct Token{value:i32}fn unwrap<T>(x:ManuallyDrop<T>)->T{ManuallyDrop::into_inner(x)}fn main(){let mut x=ManuallyDrop::new(Token{value:4});let field=(*x).value;let r=&*x;println!("{} {}",field,r.value);let r=&mut *x;r.value=7;let token=unwrap(x);println!("{}",token.value);let n=8i32;let wrapped=ManuallyDrop::new(&n);let returned=ManuallyDrop::into_inner(wrapped);println!("{}",*returned);let copy=ManuallyDrop::new(9i32);println!("{} {}",*copy,ManuallyDrop::into_inner(copy));}"#,
                expect: Expect::Run("4 4\n7\n8\n9 9\n"),
            },
            Case {
                name: "into_inner_moves_noncopy_wrapper",
                source: "use core::mem::ManuallyDrop;struct D;fn main(){let value=ManuallyDrop::new(D);let first=ManuallyDrop::into_inner(value);let second=ManuallyDrop::into_inner(value);}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "payload_borrow_blocks_mutation",
                source: "use core::mem::ManuallyDrop;fn main(){let mut value=ManuallyDrop::new(1i32);let shared=&*value;let exclusive=&mut *value;*exclusive=2;println!(\"{}\",*shared);}",
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "payload_type_cannot_change",
                source: "use core::mem::ManuallyDrop;fn take(value:ManuallyDrop<i32>){}fn main(){take(ManuallyDrop::new(true));}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "explicit_drop_requires_original_unsafe_authority",
                source: "use core::mem::ManuallyDrop;fn main(){let mut value=ManuallyDrop::new(1i32);ManuallyDrop::drop(&mut value);}",
                expect: Expect::Invalid("E0133"),
            },
        ],
        Path::new(&stock_rustc()),
    );
}

#[test]
fn typed_box_cleanup_preserves_original_address_and_drop_order() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check_with_compiler(
        "typed-box-in-place-drop",
        &[
            Case {
                name: "pointee_drop_observes_original_allocation_once",
                source: r#"struct Witness{address:usize,count:*mut u64,id:u64}
impl Drop for Witness{fn drop(&mut self){assert!(self as *mut Witness as usize==self.address);unsafe{*self.count=*self.count*10u64+self.id;}println!("drop {}",self.id);}}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let mut value=Box::new(Witness{address:0usize,count:count_ptr,id:1u64});
let address=&*value as *const Witness as usize;value.address=address;
drop(value);assert!(count==1u64);println!("count {}",count);}"#,
                expect: Expect::Run("drop 1\ncount 1\n"),
            },
            Case {
                name: "concrete_fn_mut_option_map_cleans_remaining_box_payload",
                source: r#"struct Witness{count:*mut u64}
impl Drop for Witness{fn drop(&mut self){unsafe{*self.count=*self.count+1u64;}println!("witness");}}
struct Pair{value:i32,witness:Witness}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let input:Option<Box<Pair>>=Some(Box::new(Pair{value:7,witness:Witness{count:count_ptr}}));
let mut sum:i32=0;let result:Option<i32>=input.map(|node|{sum+=node.value;node.value});
assert!(result.unwrap()==7);assert!(sum==7);assert!(count==1u64);println!("count {} sum {}",count,sum);}"#,
                expect: Expect::Run("witness\ncount 1 sum 7\n"),
            },
            Case {
                name: "unused_concrete_box_drop_cannot_invalidate_borrow",
                source: "struct Value{value:i32}fn unused(){let owner=Box::new(Value{value:7});let borrowed=&*owner;drop(owner);println!(\"{}\",borrowed.value);}fn main(){}",
                expect: Expect::Invalid("E0505"),
            },
            Case {
                name: "unused_concrete_box_cannot_move_after_drop",
                source: "struct Value{value:i32}fn unused(){let owner=Box::new(Value{value:7});drop(owner);let moved=owner;}fn main(){}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "nested_boxes_drop_parent_then_child_at_original_addresses",
                source: r#"struct Leaf{address:usize,count:*mut u64}
impl Drop for Leaf{fn drop(&mut self){assert!(self as *mut Leaf as usize==self.address);unsafe{*self.count=*self.count*10u64+2u64;}println!("leaf");}}
struct Parent{address:usize,count:*mut u64,child:Box<Leaf>}
impl Drop for Parent{fn drop(&mut self){assert!(self as *mut Parent as usize==self.address);unsafe{*self.count=*self.count*10u64+1u64;}println!("parent");}}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let mut leaf=Box::new(Leaf{address:0usize,count:count_ptr});let leaf_address=&*leaf as *const Leaf as usize;leaf.address=leaf_address;
let mut parent=Box::new(Parent{address:0usize,count:count_ptr,child:leaf});let parent_address=&*parent as *const Parent as usize;parent.address=parent_address;
let outer=Box::new(parent);drop(outer);assert!(count==12u64);println!("count {}",count);}"#,
                expect: Expect::Run("parent\nleaf\ncount 12\n"),
            },
            Case {
                name: "wide_payload_drops_in_place_before_release",
                source: r#"struct Wide{address:usize,count:*mut u64,payload:[u64;1025]}
impl Drop for Wide{fn drop(&mut self){assert!(self as *mut Wide as usize==self.address);assert!(self.payload[0]==7u64);assert!(self.payload[1024]==9u64);unsafe{*self.count=*self.count+1u64;}println!("wide");}}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let mut value=Box::new(Wide{address:0usize,count:count_ptr,payload:[7u64;1025]});
let address=&*value as *const Wide as usize;value.address=address;value.payload[1024]=9u64;
drop(value);assert!(count==1u64);println!("count {}",count);}"#,
                expect: Expect::Run("wide\ncount 1\n"),
            },
            Case {
                name: "zero_sized_pointee_destructor_and_trivial_box",
                source: r#"struct Zero;impl Drop for Zero{fn drop(&mut self){println!("zero");}}
fn main(){drop(Box::new(Zero));drop(Box::new(()));let value=Box::new(7u64);assert!(*value==7u64);drop(value);println!("done");}"#,
                expect: Expect::Run("zero\ndone\n"),
            },
            Case {
                name: "noncopy_move_out_drops_only_the_returned_value",
                source: r#"struct Witness{count:*mut u64}
impl Drop for Witness{fn drop(&mut self){unsafe{*self.count=*self.count+1u64;}println!("witness");}}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let owner=Box::new(Witness{count:count_ptr});let moved=*owner;assert!(count==0u64);drop(moved);assert!(count==1u64);println!("count {}",count);}"#,
                expect: Expect::Run("witness\ncount 1\n"),
            },
            Case {
                name: "dynamic_box_cleanup_keeps_existing_vtable_protocol",
                source: r#"trait Value{fn value(&self)->i32;}
struct Witness{count:*mut u64}
impl Value for Witness{fn value(&self)->i32{7}}
impl Drop for Witness{fn drop(&mut self){unsafe{*self.count=*self.count+1u64;}println!("dynamic");}}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let owner:Box<dyn Value>=Box::new(Witness{count:count_ptr});assert!(owner.value()==7);drop(owner);assert!(count==1u64);println!("count {}",count);}"#,
                expect: Expect::Run("dynamic\ncount 1\n"),
            },
        ],
        Path::new(&stock_rustc()),
    );
}

#[test]
fn ordinary_box_source_applications_do_not_require_recursive_thunk_identity() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check_with_compiler(
        "ordinary-box-source-drop",
        &[Case {
            name: "repeated_source_views_and_distinct_actuals_keep_exact_cleanup",
            source: r#"struct Guard<T>{value:T,digit:u64,count:*mut u64}
impl<T> Drop for Guard<T>{fn drop(&mut self){unsafe{*self.count=*self.count*10u64+self.digit;}println!("guard {}",self.digit);}}
fn boxed<T>(value:T)->Box<T>{Box::new(value)}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let first=boxed(Guard{value:7u64,digit:1u64,count:count_ptr});drop(first);
let second=boxed(Guard{value:[8u64;2],digit:2u64,count:count_ptr});drop(second);
let third=boxed(Guard{value:9u64,digit:3u64,count:count_ptr});drop(third);
assert!(count==123u64);println!("count {}",count);}"#,
            expect: Expect::Run("guard 1\nguard 2\nguard 3\ncount 123\n"),
        }],
        Path::new(&stock_rustc()),
    );
}

#[test]
fn recursive_generic_box_owners_keep_distinct_actuals_and_instance_scopes() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check_with_compiler(
        "recursive-generic-box-scopes",
        &[Case {
            name: "distinct_recursive_actuals_in_one_function_and_repeated_generic_instances",
            source: r#"struct Token{digit:u64,count:*mut u64}
impl Drop for Token{fn drop(&mut self){unsafe{*self.count=*self.count*10u64+self.digit;}println!("token {}",self.digit);}}
struct Node<T>{value:T,next:Option<Box<Node<T>>>}
fn consume<T>(value:Box<Node<T>>){drop(value);}
fn main(){let mut count=0u64;let ptr=&mut count as *mut u64;
{let first=Box::new(Node{value:Token{digit:1u64,count:ptr},next:None});
let second=Box::new(Node{value:[Token{digit:2u64,count:ptr};1],next:None});
drop(first);drop(second);}
let third=Box::new(Node{value:Token{digit:3u64,count:ptr},next:None});consume(third);
let fourth=Box::new(Node{value:[Token{digit:4u64,count:ptr};1],next:None});consume(fourth);
assert!(count==1234u64);println!("count {}",count);}"#,
            expect: Expect::Run("token 1\ntoken 2\ntoken 3\ntoken 4\ncount 1234\n"),
        }],
        Path::new(&stock_rustc()),
    );
}

#[test]
fn recursive_typed_boxes_drop_in_place_once_and_preserve_partial_moves() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check_with_compiler(
        "recursive-typed-box-drop",
        &[
            Case {
                name: "recursive_nodes_run_own_drop_at_each_original_address",
                source: r#"struct Node{address:usize,digit:u64,count:*mut u64,next:Option<Box<Node>>}
impl Drop for Node{fn drop(&mut self){assert!(self as *mut Node as usize==self.address);unsafe{*self.count=*self.count*10u64+self.digit;}println!("node {}",self.digit);}}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let mut last=Box::new(Node{address:0usize,digit:3u64,count:count_ptr,next:None});let address=&*last as *const Node as usize;last.address=address;
let mut middle=Box::new(Node{address:0usize,digit:2u64,count:count_ptr,next:Some(last)});let address=&*middle as *const Node as usize;middle.address=address;
let mut first=Box::new(Node{address:0usize,digit:1u64,count:count_ptr,next:Some(middle)});let address=&*first as *const Node as usize;first.address=address;
drop(first);assert!(count==123u64);println!("count {}",count);}"#,
                expect: Expect::Run("node 1\nnode 2\nnode 3\ncount 123\n"),
            },
            Case {
                name: "mutually_recursive_aggregate_retains_only_live_partial_paths",
                source: r#"struct Token{digit:u64,count:*mut u64}
impl Drop for Token{fn drop(&mut self){unsafe{*self.count=*self.count*10u64+self.digit;}println!("token {}",self.digit);}}
struct A{token:Token,next:Option<Box<B>>}struct B{token:Token,next:Option<Box<A>>}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
{let last=Box::new(A{token:Token{digit:3u64,count:count_ptr},next:None});
let middle=Box::new(B{token:Token{digit:2u64,count:count_ptr},next:Some(last)});
let first=Box::new(A{token:Token{digit:1u64,count:count_ptr},next:Some(middle)});
let moved=first.token;drop(moved);assert!(count==1u64);}
assert!(count==123u64);println!("count {}",count);}"#,
                expect: Expect::Run("token 1\ntoken 2\ntoken 3\ncount 123\n"),
            },
        ],
        Path::new(&stock_rustc()),
    );
}

#[test]
fn raw_pointer_casts_reborrow_mutable_reference_places_without_moving_them() {
    use explicit_drop_oracle::{Case, Expect};
    explicit_drop_oracle::check_with_compiler(
        "raw-pointer-cast-reborrow",
        &[
            Case {
                name: "mutable_reference_remains_live_after_mutable_and_const_casts",
                source: r#"fn observe(r:&mut i32){let first=r as *mut i32;let second=r as *const i32;assert!(first as usize==second as usize);unsafe{*first=*first+1;}*r=*r+1;println!("{}",*r);}
fn main(){let mut value=5;observe(&mut value);assert!(value==7);}"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "raw_cast_rvalue_is_evaluated_once",
                source: r#"fn main(){let mut value=5;let mut count=0;let pointer=({count+=1;&mut value}) as *mut i32;unsafe{*pointer=*pointer+1;}assert!(count==1);assert!(value==6);println!("{} {}",value,count);}"#,
                expect: Expect::Run("6 1\n"),
            },
            Case {
                name: "real_reference_move_still_invalidates_the_old_carrier",
                source: "fn main(){let mut value=5;let r=&mut value;let moved=r;let pointer=r as *mut i32;*moved=7;}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "cast_reborrow_cannot_override_a_live_shared_loan",
                source: "fn main(){let mut value=5;let r=&mut value;let shared=&*r;let pointer=r as *mut i32;println!(\"{}\",*shared);}",
                expect: Expect::Invalid("E0502"),
            },
        ],
        Path::new(&stock_rustc()),
    );
}

#[test]
fn typed_box_native_into_inner_sentinel_and_exact_mapping_extent() {
    // The native Box::into_inner extension and private mmap extent are tested
    // separately from the unchanged-source stock differential cases above.
    let source = r#"struct Token{count:*mut u64}
impl Drop for Token{fn drop(&mut self){unsafe{*self.count=*self.count+1u64;}}}
fn main(){let mut count=0u64;let count_ptr=&mut count as *mut u64;
let value=Box::into_inner(Box::new(Token{count:count_ptr}));assert!(count==0u64);drop(value);assert!(count==1u64);
let owner=Box::new([7u64;1025]);let address=&*owner as *const [u64;1025] as usize;
assert!(address%4096usize==0usize);let last_page=address+8192usize;
let mut resident=0u8;let resident_ptr=&mut resident as *mut u8 as usize;
let before=unsafe{__justrust_linux_syscall6!(27usize,last_page,4096usize,resident_ptr,0usize,0usize,0usize)};
assert!(before==0usize);drop(owner);
let after=unsafe{__justrust_linux_syscall6!(27usize,last_page,4096usize,resident_ptr,0usize,0usize,0usize)};
assert!(after==18446744073709551604usize);println!("native box cleanup");}"#;
    for workers in [1, 8] {
        for check in [true, false] {
            let context = format!("typed Box native protocol workers={workers} CHECK={check}");
            let compiled = compiler::compile(source.into(), workers, check)
                .unwrap_or_else(|error| panic!("{context}: {error}"));
            assert_eq!(compiled.executable.is_none(), check, "{context}");
            if check {
                assert!(
                    compiled
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130 | 131 | 133)),
                    "{context}: CHECK demanded code generation"
                );
            } else {
                let scratch = Scratch::new();
                let path = scratch.write("native-box", &compiled.executable.unwrap());
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                let ran = capture(&mut Command::new(path), &scratch, 20);
                successful(&ran, &context);
                assert_eq!(ran.stdout, b"native box cleanup\n", "{context}");
            }
        }
    }
}

const INDEX: &str = include_str!("fixtures/drop_normal/receipts-v9/fixture-index.tsv");
const MANIFEST_HASH: &str = "afc7b27b95496aa84316d9e21485fb7c4f2c18d57701251bcffc152593aee7fe";
const INDEX_HASH: &str = "213e1c312094ffb078cd49c04e508721188e4b0308b1c990b939ce3af5b585e7";
const RECEIPT_HASH: &str = "a35fe212113376b84ed3b011f149b72a15001d2c78c76e3d38acd5e6427c45c1";
/// Byte hash of the exact stock rustc 1.98.1 (commit 48a229cea) binaries the
/// receipts were measured with: the Arch Linux package build and the official
/// rustup x86_64-unknown-linux-gnu build installed on hosted build machines.
/// Both report the same version, commit and LLVM 22.1.8.
const RUSTC_HASHES: &[&str] = &[
    "c916a85b7d4f52d9650399eebd15c672377933e6c2d8e98215757ab0b9b37136",
    "859254978c0a0402c32f949f6de0d99aee73be8d15f45aac00ae1448aac51e74",
];
const CHILD: &str = "JR_DROP_NORMAL_VERIFIED_STOCK_CHILD";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/drop_normal")
            });
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for _ in 0..100 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "run-drop-normal-{}-{epoch}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create scratch: {e}"),
            }
        }
        panic!("could not create exclusive scratch")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        new_file(&path).write_all(bytes).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap()
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl std::fmt::Display for Captured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "status: {}\nstdout:\n{}\nstderr:\n{}",
            self.status,
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        )
    }
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&stdout))
        .stderr(new_file(&stderr));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|e| panic!("{command:?}: {e}")),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(seconds) {
            let _ = child.0.kill();
            let _ = child.0.wait();
            panic!(
                "timed out: {command:?}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&fs::read(&stdout).unwrap()),
                String::from_utf8_lossy(&fs::read(&stderr).unwrap()),
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Captured {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}
fn successful(result: &Captured, context: &str) {
    assert!(result.status.success(), "{context}: {result}");
}
fn rejected(result: &Captured, context: &str) {
    assert_eq!(
        result.status.code(),
        Some(1),
        "{context}: expected normal rejection: {result}"
    );
    assert!(
        !result.stderr.is_empty(),
        "{context}: missing diagnostic: {result}"
    );
}
fn check_hash(path: &Path, expected: &str, scratch: &Scratch) {
    let result = capture(Command::new("sha256sum").arg(path), scratch, 10);
    successful(&result, "frozen content hash");
    let output = String::from_utf8(result.stdout).unwrap();
    assert_eq!(
        output.split_whitespace().next(),
        Some(expected),
        "{}",
        path.display()
    );
}
fn unhex(value: &str) -> Vec<u8> {
    if value == "-" {
        return Vec::new();
    }
    assert_eq!(value.len() % 2, 0);
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
        .collect()
}
struct Case {
    category: String,
    path: String,
    hash: String,
    source: String,
    compile_status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/drop_normal")
}
fn cases() -> Vec<Case> {
    assert_eq!(
        INDEX.lines().next(),
        Some(format!("# manifest_sha256={MANIFEST_HASH}").as_str())
    );
    INDEX
        .lines()
        .skip(1)
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 7);
            let invalid = fields[0] == "negative";
            assert!(matches!(fields[0], "positive" | "negative" | "unsupported"));
            assert_eq!(fields[3], if invalid { "1" } else { "0" });
            assert_eq!(fields[4], if invalid { "none" } else { "0" });
            Case {
                category: fields[0].into(),
                path: fields[1].into(),
                hash: fields[2].into(),
                source: fs::read_to_string(corpus_root().join(fields[1])).unwrap(),
                compile_status: fields[3].parse().unwrap(),
                stdout: unhex(fields[5]),
                stderr: unhex(fields[6]),
            }
        })
        .collect()
}
fn fixture(case: &Case) -> Scratch {
    let scratch = Scratch::new();
    let source = scratch.write("main.rs", case.source.as_bytes());
    check_hash(&source, &case.hash, &scratch);
    scratch
}
fn execute(path: &Path, case: &Case, scratch: &Scratch, context: &str) {
    let bytes = fs::read(path).unwrap();
    assert!(bytes.len() >= 64, "{context}: truncated ELF");
    assert_eq!(&bytes[..6], b"\x7fELF\x02\x01", "{context}");
    let result = capture(&mut Command::new(path), scratch, 10);
    successful(&result, context);
    assert_eq!(
        result.stdout, case.stdout,
        "{context}: exact stdout mismatch\n{result}"
    );
    assert_eq!(
        result.stderr, case.stderr,
        "{context}: exact stderr mismatch\n{result}"
    );
}
fn stock(case: &Case) {
    let scratch = fixture(case);
    for check in [true, false] {
        let output = scratch.path(if check { "stock.rmeta" } else { "stock" });
        let mut command = Command::new(stock_rustc());
        command
            .arg(scratch.path("main.rs"))
            .args([
                "--crate-name",
                "drop_fixture",
                "--edition=2024",
                "-C",
                "panic=abort",
                "-A",
                "warnings",
            ])
            .arg("-o")
            .arg(output);
        if check {
            command.arg("--emit=metadata");
        }
        let result = capture(&mut command, &scratch, 60);
        assert_eq!(
            result.status.code(),
            Some(case.compile_status),
            "{}: {result}",
            case.path
        );
        if case.category == "negative" {
            let expected = match case.path.as_str() {
                "negative/copy_drop.rs" | "negative/copy_drop_manual.rs" => "E0184",
                "negative/move_field.rs" => "E0509",
                _ => "E0040",
            };
            assert!(
                String::from_utf8_lossy(&result.stderr).contains(expected),
                "{}: {result}",
                case.path
            );
        }
        if check || case.category == "negative" {
            assert!(!scratch.path("stock").exists());
        }
    }
    if case.category != "negative" {
        execute(&scratch.path("stock"), case, &scratch, "stock runtime");
    }
}
fn native_accepted(case: &Case) -> bool {
    (case.category == "positive"
        || matches!(
            case.path.as_str(),
            "unsupported/borrow.rs"
                | "unsupported/generic_function.rs"
                | "unsupported/dead_generic.rs"
                | "unsupported/array.rs"
                | "unsupported/borrowed_replacement.rs"
                | "unsupported/dead_aggregate.rs"
                | "unsupported/enum.rs"
                | "unsupported/generic.rs"
                | "unsupported/nested.rs"
                | "unsupported/tuple.rs"
                | "unsupported/reference_field.rs"
                | "unsupported/owned_match.rs"
                | "unsupported/owned_if_let.rs"
        ))
        && !path_blocked(case)
}
/// Unqualified `module::Trait::method` / `Trait::method` paths are existing
/// resolver boundaries unrelated to Drop. Frozen qualified variants carry the
/// native claims instead; a resolver rejection never counts as E0040
/// conformance. `parameter_paths.rs` (a `mut` by-value parameter) is native
/// positive since MUT-PARAMS support.
fn path_blocked(case: &Case) -> bool {
    matches!(case.path.as_str(), "negative/explicit_ufcs.rs")
}
fn rejection_reason(message: &str, case: &Case) {
    let lower = message.to_ascii_lowercase();
    for forbidden in [
        "resource",
        "budget",
        "allocation failed",
        "out of memory",
        "contract:",
        "output already exists",
        "no such file",
        "permission denied",
        "internal",
        "panicked",
    ] {
        assert!(
            !lower.contains(forbidden),
            "{}: infrastructure failure is not source rejection: {message}",
            case.path
        );
    }
    if path_blocked(case) {
        assert!(
            lower.contains("non-module declaration in path")
                || lower.contains("unresolved function"),
            "{}: expected explicit parser/resolver boundary: {message}",
            case.path
        );
        return;
    }
    // Stock E0509 requires a non-Copy field, outside the scalar owner domain.
    // This case checks an explicit unsupported owner/type boundary, not native
    // field-move language conformance or diagnostic precedence.
    if case.category == "unsupported" {
        assert!(
            lower.contains("unsupported"),
            "{}: excluded domain requires explicit unsupported diagnostic: {message}",
            case.path
        );
        let relevant = lower.contains("drop") || lower.contains("destructor");
        assert!(
            relevant,
            "{}: unrelated unsupported boundary: {message}",
            case.path
        );
        return;
    }
    let expected = match case.path.as_str() {
        "negative/copy_drop_manual.rs" | "negative/copy_drop.rs" => {
            lower.contains("copy") && lower.contains("drop") && lower.contains("same type")
        }
        "negative/move_field.rs" => lower.contains("e0509"),
        "negative/explicit_method.rs"
        | "negative/explicit_qualified.rs"
        | "negative/generic_explicit_ufcs.rs" => {
            (lower.contains("explicit") || lower.contains("direct"))
                && (lower.contains("drop") || lower.contains("destructor"))
        }
        _ => false,
    };
    assert!(
        expected,
        "{}: expected specific semantic rejection, got {message}",
        case.path
    );
    assert!(
        !lower.contains("unsupported"),
        "{}: unsupported is not invalid-Rust conformance: {message}",
        case.path
    );
}
fn native(case: &Case) {
    let accepted = native_accepted(case);
    eprintln!("native case {} accepted={accepted}", case.path);
    for workers in [1, 8] {
        for check in [true, false] {
            let scratch = fixture(case);
            let context = format!("{} workers={workers} check={check}", case.path);
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .arg(scratch.path("main.rs"))
                .args(["--workers", &workers.to_string()]);
            if check {
                command.arg("--check");
            } else {
                command.arg("-o").arg(scratch.path("native"));
            }
            let cli = capture(&mut command, &scratch, 30);
            if accepted {
                successful(&cli, &context);
            } else {
                rejected(&cli, &context);
                rejection_reason(&String::from_utf8_lossy(&cli.stderr), case);
            }
            if check || !accepted {
                assert!(
                    !scratch.path("native").exists(),
                    "{context}: executable leaked"
                );
            } else {
                execute(&scratch.path("native"), case, &scratch, &context);
            }
            let observed = compiler::compile_observed(case.source.clone(), workers, check);
            if accepted {
                let compiled = observed.unwrap_or_else(|e| panic!("{context}: {e}"));
                assert!(compiled.report.capture_complete, "{context}");
                assert_eq!(compiled.report.events_dropped, 0, "{context}");
                assert_eq!(compiled.report.failures, 0, "{context}");
                assert!(compiled.analyses.iter().all(|a| a.passed()), "{context}");
                assert_eq!(compiled.analyses.len(), compiled.functions, "{context}");
                assert_eq!(compiled.control_flow.len(), compiled.functions, "{context}");
                assert!(
                    compiled.resource_usage.first_failure.is_none(),
                    "{context}: unexpected budget failure"
                );
                assert!(
                    !compiled.resource_usage.poisoned,
                    "{context}: poisoned successful compilation"
                );
                assert_eq!(compiled.executable.is_none(), check, "{context}");
                if check {
                    assert!(
                        !compiled
                            .report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 103),
                        "{context}: CHECK evaluated machine-code node"
                    );
                }
                if let Some(bytes) = compiled.executable {
                    let path = scratch.write("observed", &bytes);
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                    execute(&path, case, &scratch, &context);
                }
            } else {
                let failure = observed
                    .err()
                    .unwrap_or_else(|| panic!("{context}: unexpected observed acceptance"));
                rejection_reason(&failure.to_string(), case);
                assert!(
                    failure.resource_usage.first_failure.is_none(),
                    "{context}: budget exhaustion is not semantic rejection"
                );
                assert!(
                    !failure.resource_usage.poisoned,
                    "{context}: poisoned rejection ledger"
                );
                if let Some(report) = failure.report {
                    assert!(report.capture_complete, "{context}");
                    assert_eq!(report.events_dropped, 0, "{context}");
                }
            }
        }
    }
}

#[test]
fn frozen_stock_before_native_normal_drop() {
    let cases = cases();
    assert_eq!(cases.len(), 56);
    assert_eq!(
        cases.iter().filter(|c| c.category == "positive").count(),
        33
    );
    assert_eq!(cases.iter().filter(|c| c.category == "negative").count(), 7);
    assert_eq!(
        cases.iter().filter(|c| c.category == "unsupported").count(),
        16
    );
    assert_eq!(
        cases.iter().filter(|case| native_accepted(case)).count(),
        46
    );
    if std::env::var(CHILD).ok().as_deref() == Some(MANIFEST_HASH) {
        // Stock has already checked the entire corpus before this child starts.
        // Cases own disjoint scratch directories and compiler graphs, so bounded
        // parallel lanes preserve every workers-1/8 check without making the
        // child deadline a limit on the sum of unrelated compilation times.
        for accepted in [true, false] {
            let selected: Vec<_> = cases
                .iter()
                .filter(|case| native_accepted(case) == accepted)
                .collect();
            std::thread::scope(|scope| {
                let handles: Vec<_> = (0..4)
                    .map(|lane| {
                        let selected = &selected;
                        scope.spawn(move || {
                            for case in selected.iter().skip(lane).step_by(4) {
                                native(case);
                            }
                        })
                    })
                    .collect();
                for handle in handles {
                    if let Err(error) = handle.join() {
                        std::panic::resume_unwind(error);
                    }
                }
            });
        }
        return;
    }
    let scratch = Scratch::new();
    let root = corpus_root();
    check_hash(
        &root.join("receipts-v9/manifest.json"),
        MANIFEST_HASH,
        &scratch,
    );
    check_hash(
        &root.join("receipts-v9/fixture-index.tsv"),
        INDEX_HASH,
        &scratch,
    );
    check_hash(&root.join("receipts-v9/stock.json"), RECEIPT_HASH, &scratch);
    let rustc_hash = capture(Command::new("sha256sum").arg(stock_rustc()), &scratch, 10);
    successful(&rustc_hash, "stock compiler content hash");
    let rustc_hash = String::from_utf8(rustc_hash.stdout).unwrap();
    assert!(
        RUSTC_HASHES.contains(&rustc_hash.split_whitespace().next().unwrap_or("")),
        "unrecognized stock rustc build: {rustc_hash}"
    );
    let version = capture(Command::new(stock_rustc()).arg("-Vv"), &scratch, 10);
    successful(&version, "stock compiler identity");
    assert!(
        String::from_utf8_lossy(&version.stdout)
            .contains("48a229ceaefd4985c50990b14116b6d856af0985")
    );
    // Run the complete exact-source stock corpus before native assertions, not
    // merely one stock case followed by that case's native execution.
    for case in &cases {
        stock(case);
    }
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "frozen_stock_before_native_normal_drop",
            "--nocapture",
        ])
        .env(CHILD, MANIFEST_HASH);
    let result = capture(&mut child, &scratch, 360);
    successful(
        &result,
        "bounded native CLI and observed CHECK/BUILD corpus child",
    );
}

/// Edition 2024 `if let` scrutinee temporaries end after the selected branch,
/// or before `else`; `match` keeps them for the whole expression.
#[test]
fn if_let_scrutinee_temporaries_follow_stock_2024_order() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "if-let-temporaries",
        &[Case {
            name: "then_else_match_and_value",
            source: r#"struct D(u32);
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.0); } }
impl D { fn some(&self) -> Option<u32> { Some(self.0) } fn none(&self) -> Option<u32> { None } }
fn main() {
    if let Some(v) = D(1).some() { println!("then {}", v); } else { println!("else"); }
    println!("a");
    if let Some(v) = D(2).none() { println!("then {}", v); } else { println!("else"); }
    println!("b");
    match D(3).some() { Some(v) => println!("arm {}", v), None => println!("none") }
    println!("c");
    let x = if let Some(v) = D(4).some() { v } else { 0 };
    println!("x {}", x);
    let y = if let Some(v) = D(5).none() { v } else { 9 };
    println!("y {}", y);
}"#,
            expect: Expect::Run(
                "then 1\ndrop 1\na\ndrop 2\nelse\nb\narm 3\ndrop 3\nc\ndrop 4\nx 4\ndrop 5\ny 9\n",
            ),
        }],
    );
}
