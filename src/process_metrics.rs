pub fn resident_set_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_resident_set_kib(&status)
}

fn parse_resident_set_kib(status: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value
            .split_whitespace()
            .next()
            .and_then(|amount| amount.parse().ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_status_rss() {
        assert_eq!(
            parse_resident_set_kib("Name:\tchelotype\nVmRSS:\t  123456 kB\n"),
            Some(123456)
        );
    }
}
