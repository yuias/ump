//! Header utilities.

pub fn format_duration(secs: f64) -> String {
    let total = secs as u64;
    let min = total / 60;
    let sec = total % 60;
    format!("{:02}:{:02}", min, sec)
}
