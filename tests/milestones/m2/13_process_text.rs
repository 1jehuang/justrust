#[derive(Debug)] struct Record { name: String, dept: String, salary: u32, years: u8 }
const DATA: &str = "name,dept,salary,years\nalice,eng,120000,5\nbob,ops,85000,2\ncarol,eng,135000,8\ndan,sales,70000,1\nerin,ops,92000,6\nbad,line\nfrank,eng,99000,x";
fn parse(line: &str) -> Result<Record, String> {
    let f: Vec<&str> = line.split(',').collect();
    if f.len() != 4 { return Err(format!("bad field count {}", f.len())); }
    Ok(Record { name: f[0].to_string(), dept: f[1].to_string(), salary: f[2].parse().map_err(|e| format!("{e}"))?, years: f[3].parse().map_err(|e: std::num::ParseIntError| e.to_string())? })
}
fn main() {
    let (ok, bad): (Vec<_>, Vec<_>) = DATA.lines().skip(1).map(parse).partition(Result::is_ok);
    let recs: Vec<Record> = ok.into_iter().map(Result::unwrap).collect();
    for e in bad.into_iter().map(|r| r.unwrap_err()) { println!("error: {e}"); }
    let mut depts: Vec<&str> = recs.iter().map(|r| r.dept.as_str()).collect(); depts.sort(); depts.dedup();
    for d in &depts {
        let rs: Vec<&Record> = recs.iter().filter(|r| r.dept == *d).collect();
        let avg = rs.iter().map(|r| r.salary as f64).sum::<f64>() / rs.len() as f64;
        let senior = rs.iter().max_by_key(|r| r.years).unwrap();
        println!("{d:<6} n={} avg={avg:>10.1} senior={}", rs.len(), senior.name);
    }
    let top: Vec<String> = { let mut v: Vec<&Record> = recs.iter().collect(); v.sort_by(|a, b| b.salary.cmp(&a.salary)); v.iter().take(2).map(|r| r.name.to_uppercase()).collect() };
    println!("{}", top.join(" & "));
    println!("{:?}", recs.last());
}
