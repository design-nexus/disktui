pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || text == "0" || text.eq_ignore_ascii_case("max") {
        return Some(0);
    }
    let (number, unit) = text
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic())
        .map(|(i, _)| text.split_at(i))
        .unwrap_or((text, ""));
    let number: f64 = number.trim().parse().ok()?;
    if number < 0.0 {
        return None;
    }
    let mult: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1000,
        "m" | "mb" | "mib" => 1000 * 1000,
        "g" | "gb" | "gib" => 1000 * 1000 * 1000,
        "t" | "tb" | "tib" => 1000 * 1000 * 1000 * 1000,
        _ => return None,
    };
    Some((number * mult as f64).round() as u64)
}

pub fn kernel_name(device: &str) -> &str {
    device.rsplit('/').next().unwrap_or(device)
}

pub fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

pub fn with_nul(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

pub fn brief(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    line.chars().take(160).collect()
}

pub fn current_ids() -> (u32, u32) {
    unsafe { (libc::geteuid(), libc::getegid()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_round_trip() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(parse_size("1.5G"), Some(1_500_000_000));
        assert_eq!(parse_size("max"), Some(0));
        assert_eq!(parse_size("512M"), Some(512_000_000));
        assert_eq!(kernel_name("/dev/nvme0n1p2"), "nvme0n1p2");
        assert_eq!(c_string(b"UUID=abc\0junk"), "UUID=abc");
    }
}
