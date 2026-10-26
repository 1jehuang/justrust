//! Deliberately failing libtest workload for measuring failure confirmation.
//! The passing test creates a second historical shard. All failed tests must
//! still run and remain failed, regardless of the retry strategy.

#[cfg(test)]
mod tests {
    macro_rules! failing_tests {
        ($($name:ident),+ $(,)?) => {$(
            #[test]
            fn $name() {
                std::thread::sleep(std::time::Duration::from_millis(200));
                panic!("intentional failure for retry-overhead measurement");
            }
        )+};
    }

    failing_tests!(
        fail_00, fail_01, fail_02, fail_03, fail_04, fail_05, fail_06, fail_07, fail_08, fail_09,
        fail_10, fail_11, fail_12, fail_13, fail_14, fail_15, fail_16, fail_17, fail_18, fail_19,
        fail_20, fail_21, fail_22, fail_23, fail_24, fail_25, fail_26, fail_27, fail_28, fail_29,
        fail_30, fail_31,
    );

    #[test]
    fn passing_shard() {
        std::hint::black_box(1);
    }
}
