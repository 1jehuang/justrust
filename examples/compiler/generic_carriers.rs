fn replace<T: Copy>(destination: &mut T, incoming: T) -> T {
    *destination = incoming;
    *destination
}

fn put<T>(destination: &mut T, incoming: T) {
    *destination = incoming;
}

fn run() -> i32 {
    let first = 11;
    let second = 31;
    let mut shared = &first;
    let loaded = replace::<&i32>(&mut shared, &second);
    let observed = *loaded;

    let mut original = 1;
    let mut replacement = 2;
    let mut unique = &mut original;
    put::<&mut i32>(&mut unique, &mut replacement);
    *unique = 41;

    observed + replacement
}

fn main() {
    if run() == 72 {
        println!("generic carriers ok");
    }
}
