//! Turning numbers and times into the text the window shows.

/// `1234.5` → `"1,234.50"`.
pub fn money(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let s = format!("{:.2}", v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{}{grouped}.{frac}", if v < 0.0 { "-" } else { "" })
}

/// `56_997_299` → `"57.0M"`.
pub fn volume(v: u64) -> String {
    let v = v as f64;
    match v {
        v if v >= 1e9 => format!("{:.1}B", v / 1e9),
        v if v >= 1e6 => format!("{:.1}M", v / 1e6),
        v if v >= 1e3 => format!("{:.1}K", v / 1e3),
        v => format!("{v}"),
    }
}

/// The change from `previous` to `now`: `"+1.25 (+0.42%)"`.
pub fn change(now: f64, previous: f64) -> String {
    if previous == 0.0 || !now.is_finite() || !previous.is_finite() {
        return String::new();
    }
    let d = now - previous;
    let sign = if d >= 0.0 { "+" } else { "-" };
    format!(
        "{sign}{} ({sign}{:.2}%)",
        money(d.abs()),
        (d / previous * 100.0).abs()
    )
}

/// Unix seconds → `"2026-11-24"` (UTC: trading days are dates, not moments).
pub fn date(unix: i64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// SEC's fiscal year end `"0925"` → `"September 25"`.
pub fn month_day(mmdd: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let (Some(m), Some(d)) = (mmdd.get(..2), mmdd.get(2..4)) else {
        return mmdd.to_owned();
    };
    match (m.parse::<usize>(), d.parse::<u32>()) {
        (Ok(m @ 1..=12), Ok(d)) => format!("{} {d}", MONTHS[m - 1]),
        _ => mmdd.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_volume() {
        assert_eq!(money(1234.5), "1,234.50");
        assert_eq!(money(349.08), "349.08");
        assert_eq!(money(-1_000_000.0), "-1,000,000.00");
        assert_eq!(money(f64::NAN), "—");
        assert_eq!(volume(56_997_299), "57.0M");
        assert_eq!(volume(2_500_000_000), "2.5B");
        assert_eq!(volume(950), "950");
    }

    #[test]
    fn change_has_a_sign() {
        assert_eq!(change(110.0, 100.0), "+10.00 (+10.00%)");
        assert_eq!(change(95.0, 100.0), "-5.00 (-5.00%)");
        assert_eq!(change(1.0, 0.0), "");
    }

    #[test]
    fn dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_795_996_800), "2026-11-30");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(month_day("0925"), "September 25");
        assert_eq!(month_day("1231"), "December 31");
        assert_eq!(month_day(""), "");
    }
}
