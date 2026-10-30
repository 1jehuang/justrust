pub fn through_registry(n: u32) -> String {
    format!("{} from {}", dep::scale(n), dep::origin())
}
