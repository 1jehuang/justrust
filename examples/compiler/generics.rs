struct Parcel<T> {
    value: T,
    stamp: u64,
}

enum Packet<T> {
    Item(T),
}

fn identity<T>(value: T) -> T {
    value
}

fn wrap<T>(value: T) -> Parcel<T> {
    Parcel::<T> {
        value,
        stamp: 18446744073709551615u64,
    }
}

fn unwrap<T>(parcel: Parcel<T>) -> T {
    parcel.value
}

fn unpack<T>(packet: Packet<T>) -> T {
    match packet {
        Packet::<T>::Item(value) => value,
    }
}

fn relay<T>(value: T, count: i32) -> T {
    if count == 0 {
        value
    } else {
        relay::<T>(value, count - 1)
    }
}

fn pair<T, U>(left: T, right: U) -> (T, U) {
    (left, right)
}

fn valid_unused<T>(value: T) -> T {
    value
}

fn main() {
    let wide = unwrap::<u64>(unpack::<Parcel<u64>>(Packet::<Parcel<u64>>::Item(wrap::<
        u64,
    >(
        relay::<u64>(18446744073709551615u64, 3),
    ))));
    let narrow = unwrap::<u8>(wrap::<u8>(identity::<u8>(255u8)));
    let values = pair::<u64, u8>(wide, narrow);
    if values.0 == 18446744073709551615u64 && values.1 == 255u8 {
        println!("generic instances compiled correctly");
    }
}
