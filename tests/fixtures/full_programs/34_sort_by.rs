#[derive(Debug)]
struct Emp { name: String, age: u32, salary: f64 }
fn main() {
    let mut es = vec![
        Emp { name: "bob".into(), age: 30, salary: 100.0 },
        Emp { name: "al".into(), age: 25, salary: 150.5 },
        Emp { name: "cy".into(), age: 35, salary: 90.0 },
    ];
    es.sort_by(|a, b| a.age.cmp(&b.age));
    for e in &es { println!("{} {}", e.name, e.age); }
    es.sort_by(|a, b| b.salary.partial_cmp(&a.salary).unwrap());
    println!("{:?}", es.iter().map(|e| &e.name).collect::<Vec<_>>());
    let oldest = es.iter().max_by_key(|e| e.age).unwrap();
    println!("{}", oldest.name);
    let mut nums = vec![5, 3, 9, 1];
    nums.sort_unstable_by(|a, b| b.cmp(a));
    println!("{:?}", nums);
}
