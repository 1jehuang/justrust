use crate::math::sequence as series;

mod math {
    fn increment(value: i32) -> i32 {
        value + 1
    }

    pub mod sequence {
        pub fn sum(n: i32) -> i32 {
            if n == 0 {
                0
            } else {
                n + self::sum(n - 1)
            }
        }

        pub fn next_sum(n: i32) -> i32 {
            self::sum(super::increment(n))
        }
    }
}

fn main() {
    if series::next_sum(4) == 15 {
        println!("modules compiled correctly");
    } else {
        println!("incorrect module result");
    }
}
