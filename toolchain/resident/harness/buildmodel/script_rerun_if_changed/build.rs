use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=table.txt");
    let rows = fs::read_to_string("table.txt").unwrap();
    let items: Vec<String> = rows.lines().map(|it| format!("{:?}", it)).collect();
    let code = format!("pub const TABLE: [&str; {}] = [{}];\npub fn count() -> usize {{ TABLE.len() }}\n", items.len(), items.join(", "));
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("table.rs"), code).unwrap();
}
