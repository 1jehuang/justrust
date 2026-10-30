use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(feature = "debug_trace")] {
        macro_rules! trace {
            ( @chars $action_descr:expr $(, $ch:expr)* ) => {
                println!(">{}", $action_descr);
            };
            ( @noop ) => ( println!("NOOP"); );
        }
    } else {
        macro_rules! trace {
            ( @$ty:ident $($args:tt)* ) => {};
            ( @$ty:ident $($args:tt),* ) => {};
        }
    }
}
