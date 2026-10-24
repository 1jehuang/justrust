fn main() {
    let mut table=String::new();
    let mut count=0;
    for value in 0..=0x10ffffu32 {
        let Some(c)=char::from_u32(value) else {continue};
        let upper:Vec<char>=c.to_uppercase().collect();
        if upper.len()==1 && upper[0]==c {continue}
        use std::fmt::Write;
        write!(table,"{:06x}{}",value,upper.len()).unwrap();
        for i in 0..3 {write!(table,"{:06x}",upper.get(i).copied().unwrap_or('\0') as u32).unwrap();}
        count+=1;
    }
    println!("// Uppercase mapping data generated from bootstrap Rust char::to_uppercase.\n// Unicode version: {:?}. Each row: scalar (6 hex), length (1), three scalars (18 hex).\nconst UPPERCASE_ROWS: usize = {}usize;\nconst UPPERCASE_DATA: &str = \"{}\";",std::char::UNICODE_VERSION,count,table);
}
