use macros::{repeat, Fields, env_tag};
#[derive(Fields)]
pub struct S { a: u8, b: u8 FIELDS }
/// ```
/// assert_eq!(app::work_times(), TIMES);
/// assert!(app::S::field_count() >= 2);
/// ```
#[repeat(TIMES)]
pub fn work() -> u32 { BODY }
#[env_tag]
pub fn tagged() {}
#[cfg(test)]
mod tests {
    #[test] fn values() {
        println!("E2E work={} times={} fields={} tag={}", super::work(), super::work_times(), super::S::field_count(), super::tag());
    }
}
