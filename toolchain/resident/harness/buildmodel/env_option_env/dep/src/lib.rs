pub fn read() -> String {
    format!("dep: {:?}", option_env!("BM_OPTIONAL"))
}
