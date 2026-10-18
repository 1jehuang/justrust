fn bubble_sort(v: &mut [i32]) {
    let n = v.len();
    for i in 0..n {
        let mut swapped = false;
        for j in 0..n - 1 - i {
            if v[j] > v[j + 1] {
                v.swap(j, j + 1);
                swapped = true;
            }
        }
        if !swapped {
            break;
        }
    }
}

fn insertion_sort(v: &mut Vec<i32>) {
    for i in 1..v.len() {
        let key = v[i];
        let mut j = i;
        while j > 0 && v[j - 1] > key {
            v[j] = v[j - 1];
            j -= 1;
        }
        v[j] = key;
    }
}

fn merge_sort(v: &[i32]) -> Vec<i32> {
    if v.len() <= 1 {
        return v.to_vec();
    }
    let mid = v.len() / 2;
    let left = merge_sort(&v[..mid]);
    let right = merge_sort(&v[mid..]);
    let mut out = Vec::with_capacity(v.len());
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        if left[i] <= right[j] {
            out.push(left[i]);
            i += 1;
        } else {
            out.push(right[j]);
            j += 1;
        }
    }
    out.extend_from_slice(&left[i..]);
    out.extend_from_slice(&right[j..]);
    out
}

fn quick_sort<T: PartialOrd + Copy>(v: &mut [T]) {
    if v.len() <= 1 {
        return;
    }
    let pivot = v[v.len() - 1];
    let mut store = 0;
    for i in 0..v.len() - 1 {
        if v[i] < pivot {
            v.swap(i, store);
            store += 1;
        }
    }
    let last = v.len() - 1;
    v.swap(store, last);
    let (left, right) = v.split_at_mut(store);
    quick_sort(left);
    quick_sort(&mut right[1..]);
}

fn lcg(seed: &mut u64) -> i32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 33) % 1000) as i32
}

fn main() {
    let mut seed = 42;
    let data: Vec<i32> = (0..20).map(|_| lcg(&mut seed)).collect();
    println!("input:     {:?}", data);
    let mut a = data.clone();
    bubble_sort(&mut a);
    println!("bubble:    {:?}", a);
    let mut b = data.clone();
    insertion_sort(&mut b);
    let c = merge_sort(&data);
    let mut d = data.clone();
    quick_sort(&mut d);
    let mut e = data.clone();
    e.sort();
    println!("all agree: {}", a == b && b == c && c == d && d == e);
    let mut f = vec![3.5, -1.0, 2.25, 0.0];
    quick_sort(&mut f);
    println!("floats: {:?}", f);
    let mut words = vec!["delta", "alpha", "charlie", "bravo"];
    words.sort_by(|x, y| y.cmp(x));
    println!("desc words: {:?}", words);
    let mut by_len = vec!["ccc", "a", "bb", "dddd"];
    by_len.sort_by_key(|s| s.len());
    println!("by len: {:?}", by_len);
}
