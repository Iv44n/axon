//! The local time's offset from UTC, read from the time zone database the way the C library reads it: `TZ`, else
//! `/etc/localtime`; a zone file is TZif (RFC 8536), a rule is POSIX (`CET-1CEST,M3.5.0,M10.5.0/3`). Our own, so the
//! clock chip needs no call into C (`localtime_r`) and no `unsafe`.
//!
//! Nothing is kept: the file is read again each time the clock is asked, on the context thread, so a zone changed
//! while axon runs is seen at the next minute.

use std::path::{Path, PathBuf};

/// Seconds east of UTC in the local zone at `t` (Unix seconds). UTC when the zone cannot be read.
pub fn offset(t: i64) -> i32 {
    let Some(tz) = std::env::var_os("TZ") else {
        return file_offset(Path::new("/etc/localtime"), t).unwrap_or(0);
    };
    let tz = tz.to_string_lossy();
    // `:name` is always a file; a bare name is a file if there is one, else a rule.
    let (name, file_only) = match tz.strip_prefix(':') {
        Some(n) => (n, true),
        None => (&*tz, false),
    };
    if name.is_empty() {
        return 0;
    }
    let path = if name.starts_with('/') {
        PathBuf::from(name)
    } else {
        std::env::var_os("TZDIR").filter(|d| !d.is_empty()).map_or_else(|| PathBuf::from("/usr/share/zoneinfo"), PathBuf::from).join(name)
    };
    file_offset(&path, t).or_else(|| if file_only { None } else { rule_offset(name, t) }).unwrap_or(0)
}

fn file_offset(path: &Path, t: i64) -> Option<i32> {
    // A zone file is a few kB; anything far larger is not one.
    let f = std::fs::read(path).ok().filter(|f| f.len() < 1 << 20)?;
    tzif_offset(&f, t)
}

fn be32(f: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_be_bytes(f.get(at..at + 4)?.try_into().ok()?))
}

fn be64(f: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_be_bytes(f.get(at..at + 8)?.try_into().ok()?))
}

/// The six counts of a TZif header at `at`: `isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt`.
fn counts(f: &[u8], at: usize) -> Option<[usize; 6]> {
    if f.get(at..at + 4)? != b"TZif" {
        return None;
    }
    let mut n = [0; 6];
    for (i, n) in n.iter_mut().enumerate() {
        *n = be32(f, at + 20 + 4 * i)? as u32 as usize;
    }
    Some(n)
}

/// Seconds east of UTC at `t` by a TZif file: the type of the last transition at or before `t`; after the last one the
/// footer's rule (version 2 and later); before the first one the first type.
pub fn tzif_offset(f: &[u8], t: i64) -> Option<i32> {
    let [isut, isstd, leap, time, typ, chars] = counts(f, 0)?;
    // Version 2 and later repeat the data with 64-bit times after the 32-bit block, and end with the rule.
    let v2 = *f.get(4)? >= b'2';
    let (at, size) = if v2 { (44 + time * 5 + typ * 6 + chars + leap * 8 + isstd + isut, 8) } else { (0, 4) };
    let [isut, isstd, leap, time, typ, chars] = if v2 { counts(f, at)? } else { [isut, isstd, leap, time, typ, chars] };
    if typ == 0 {
        return None;
    }
    let times = at + 44;
    let index = times + time * size;
    let types = index + time;
    let end = types + typ * 6 + chars + leap * (size + 4) + isstd + isut;
    let when = |i: usize| if v2 { be64(f, times + 8 * i) } else { be32(f, times + 4 * i).map(i64::from) };
    // How many transitions are at or before `t` (they are in order).
    let (mut lo, mut hi) = (0, time);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if when(mid)? <= t {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if v2 && lo == time {
        let rule = f.get(end..).and_then(|r| r.strip_prefix(b"\n")).and_then(|r| r.split(|&b| b == b'\n').next());
        if let Some(o) = rule.and_then(|r| std::str::from_utf8(r).ok()).filter(|r| !r.is_empty()).and_then(|r| rule_offset(r, t)) {
            return Some(o);
        }
    }
    let ty = if lo == 0 { 0 } else { *f.get(index + lo - 1)? as usize };
    if ty >= typ {
        return None;
    }
    be32(f, types + ty * 6)
}

/// When in the year a rule's daylight time starts or ends.
#[derive(Clone, Copy)]
enum Day {
    /// `Jn`: 1 to 365, February 29 never counted.
    Julian(u16),
    /// `n`: 0 to 365, February 29 counted.
    Zero(u16),
    /// `Mm.w.d`: day `d` (0 = Sunday) of week `w` (5 = the last) of month `m`.
    Month(u8, u8, u8),
}

/// A rule's text, read from the left.
struct Text<'a> {
    s: &'a [u8],
    at: usize,
}

impl Text<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        let hit = self.peek() == Some(c);
        self.at += hit as usize;
        hit
    }

    fn number(&mut self, max: u32) -> Option<u32> {
        let from = self.at;
        let mut n = 0u32;
        while let Some(d) = self.peek().filter(u8::is_ascii_digit) {
            n = n.checked_mul(10)? + u32::from(d - b'0');
            self.at += 1;
        }
        (self.at > from && n <= max).then_some(n)
    }

    /// A zone's name: three letters or more, or anything between `<` and `>` (`<-05>`).
    fn name(&mut self) -> Option<()> {
        if self.eat(b'<') {
            while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'-') {
                self.at += 1;
            }
            return self.eat(b'>').then_some(());
        }
        let from = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.at += 1;
        }
        (self.at - from >= 3).then_some(())
    }

    /// `[+-]hh[:mm[:ss]]` in seconds; hours up to `max_h`.
    fn clock(&mut self, max_h: u32) -> Option<i32> {
        let neg = self.eat(b'-');
        if !neg {
            self.eat(b'+');
        }
        let mut s = self.number(max_h)? * 3600;
        if self.eat(b':') {
            s += self.number(59)? * 60;
            if self.eat(b':') {
                s += self.number(59)?;
            }
        }
        Some(if neg { -(s as i32) } else { s as i32 })
    }

    fn day(&mut self) -> Option<(Day, i32)> {
        let day = if self.eat(b'J') {
            Day::Julian(self.number(365).filter(|&n| n >= 1)? as u16)
        } else if self.eat(b'M') {
            let m = self.number(12).filter(|&m| m >= 1)?;
            self.eat(b'.').then_some(())?;
            let w = self.number(5).filter(|&w| w >= 1)?;
            self.eat(b'.').then_some(())?;
            Day::Month(m as u8, w as u8, self.number(6)? as u8)
        } else {
            Day::Zero(self.number(365)? as u16)
        };
        // The local time of day of the change: 02:00 unless said, and up to ±167 hours (RFC 8536 3.3.1).
        let time = if self.eat(b'/') { self.clock(167)? } else { 7200 };
        Some((day, time))
    }
}

/// Seconds east of UTC at `t` by a POSIX rule. Its offsets count west (`CET-1` is an hour east); without a daylight
/// offset the daylight time is an hour ahead, and without dates it follows the US' rules, as in the C library.
pub fn rule_offset(rule: &str, t: i64) -> Option<i32> {
    let mut p = Text { s: rule.as_bytes(), at: 0 };
    p.name()?;
    let std_west = p.clock(24)?;
    if p.peek().is_none() {
        return Some(-std_west);
    }
    p.name()?;
    let dst_west = if p.peek().is_some_and(|c| c.is_ascii_digit() || c == b'+' || c == b'-') { p.clock(24)? } else { std_west - 3600 };
    let (start, end) = if p.eat(b',') {
        let start = p.day()?;
        p.eat(b',').then_some(())?;
        (start, p.day()?)
    } else {
        ((Day::Month(3, 2, 0), 7200), (Day::Month(11, 1, 0), 7200))
    };
    if p.peek().is_some() {
        return None;
    }
    // The year of the standard time; the start is given in standard time, the end in daylight time.
    let year = year_of((t - i64::from(std_west)).div_euclid(86_400));
    let at = |(day, time): (Day, i32), west: i32| (first_day(year, day) * 86_400 + i64::from(time)) + i64::from(west);
    let (s, e) = (at(start, std_west), at(end, dst_west));
    let dst = if s <= e { s <= t && t < e } else { t < e || t >= s };
    Some(if dst { -dst_west } else { -std_west })
}

fn leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// Days from 1970-01-01 to the date (proleptic Gregorian; H. Hinnant's `days_from_civil`).
fn days(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((i64::from(m) + 9) % 12) + 2) / 5 + i64::from(d) - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// The year of a day counted from 1970-01-01 (`civil_from_days`).
fn year_of(day: i64) -> i64 {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    yoe + era * 400 + i64::from(mp >= 10)
}

/// The day (from 1970-01-01) a rule's date falls on in `year`.
fn first_day(year: i64, day: Day) -> i64 {
    let jan1 = days(year, 1, 1);
    match day {
        Day::Julian(n) => jan1 + i64::from(n) - 1 + i64::from(leap(year) && n >= 60),
        Day::Zero(n) => jan1 + i64::from(n),
        Day::Month(m, w, d) => {
            let first = days(year, u32::from(m), 1);
            let next = if m == 12 { days(year + 1, 1, 1) } else { days(year, u32::from(m) + 1, 1) };
            // 1970-01-01 was a Thursday (4).
            let mut day = first + (i64::from(d) - (first + 4).rem_euclid(7)).rem_euclid(7) + 7 * (i64::from(w) - 1);
            while day >= next {
                day -= 7;
            }
            day
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix seconds of a UTC date and time.
    fn utc(y: i64, mo: u32, d: u32, h: i64, mi: i64) -> i64 {
        days(y, mo, d) * 86_400 + h * 3600 + mi * 60
    }

    #[test]
    fn dates_count_from_1970() {
        assert_eq!(days(1970, 1, 1), 0);
        assert_eq!(days(2000, 3, 1), 11_017);
        assert_eq!(days(1969, 12, 31), -1);
        for d in [-800_000, -1, 0, 59, 365, 11_016, 20_000, 2_000_000] {
            let y = year_of(d);
            assert!(days(y, 1, 1) <= d && d < days(y + 1, 1, 1), "day {d} is in {y}");
        }
    }

    #[test]
    fn a_zone_without_daylight_time_is_one_offset() {
        assert_eq!(rule_offset("<-05>5", 0), Some(-5 * 3600));
        assert_eq!(rule_offset("UTC0", 1_800_000_000), Some(0));
        assert_eq!(rule_offset("<+0545>-5:45", 0), Some(5 * 3600 + 45 * 60));
        assert_eq!(rule_offset("IST-5:30", 0), Some(5 * 3600 + 30 * 60));
    }

    #[test]
    fn daylight_time_starts_and_ends_on_the_rule_s_days() {
        let cet = "CET-1CEST,M3.5.0,M10.5.0/3";
        // 2026: the last Sunday of March is the 29th (01:00 UTC), of October the 25th (01:00 UTC).
        assert_eq!(rule_offset(cet, utc(2026, 1, 15, 12, 0)), Some(3600));
        assert_eq!(rule_offset(cet, utc(2026, 3, 29, 0, 59)), Some(3600));
        assert_eq!(rule_offset(cet, utc(2026, 3, 29, 1, 0)), Some(7200));
        assert_eq!(rule_offset(cet, utc(2026, 10, 25, 0, 59)), Some(7200));
        assert_eq!(rule_offset(cet, utc(2026, 10, 25, 1, 0)), Some(3600));
        let ny = "EST5EDT,M3.2.0,M11.1.0";
        // 2026: March 8 and November 1, at 02:00 local.
        assert_eq!(rule_offset(ny, utc(2026, 3, 8, 6, 59)), Some(-5 * 3600));
        assert_eq!(rule_offset(ny, utc(2026, 3, 8, 7, 0)), Some(-4 * 3600));
        assert_eq!(rule_offset(ny, utc(2026, 11, 1, 5, 59)), Some(-4 * 3600));
        assert_eq!(rule_offset(ny, utc(2026, 11, 1, 6, 0)), Some(-5 * 3600));
        assert_eq!(rule_offset("EST5EDT", utc(2026, 7, 1, 0, 0)), Some(-4 * 3600), "no dates: the US' rules");
    }

    #[test]
    fn the_south_has_daylight_time_across_the_new_year() {
        let syd = "AEST-10AEDT,M10.1.0,M4.1.0/3";
        assert_eq!(rule_offset(syd, utc(2026, 1, 10, 0, 0)), Some(11 * 3600));
        assert_eq!(rule_offset(syd, utc(2026, 7, 10, 0, 0)), Some(10 * 3600));
        assert_eq!(rule_offset(syd, utc(2026, 12, 31, 20, 0)), Some(11 * 3600));
        // Always daylight time, written as a year-long rule.
        assert_eq!(rule_offset("EST5EDT,0/0,J365/25", utc(2026, 6, 1, 0, 0)), Some(-4 * 3600));
        assert_eq!(rule_offset("<-03>3<-02>,M3.5.0/-2,M10.5.0/-1", utc(2026, 7, 1, 0, 0)), Some(-2 * 3600), "times before midnight");
    }

    #[test]
    fn a_broken_rule_is_no_rule() {
        for r in ["", "C1", "Europe/Madrid", "CET-1CEST,M3.5.0", "CET-1CEST,M13.1.0,M10.5.0", "CET-1x"] {
            assert_eq!(rule_offset(r, 0), None, "{r:?}");
        }
    }

    /// A TZif file: the version, its transitions as `(when, type)`, its types' offsets and the footer's rule.
    fn tzif(version: u8, trans: &[(i64, u8)], types: &[i32], rule: &str) -> Vec<u8> {
        let block = |f: &mut Vec<u8>, wide: bool| {
            f.extend_from_slice(b"TZif");
            f.push(version);
            f.extend_from_slice(&[0; 15]);
            for n in [0, 0, 0, trans.len(), types.len(), 4] {
                f.extend_from_slice(&(n as u32).to_be_bytes());
            }
            for &(t, _) in trans {
                if wide { f.extend_from_slice(&t.to_be_bytes()) } else { f.extend_from_slice(&(t as i32).to_be_bytes()) }
            }
            f.extend(trans.iter().map(|&(_, ty)| ty));
            for &o in types {
                f.extend_from_slice(&o.to_be_bytes());
                f.extend_from_slice(&[0, 0]);
            }
            f.extend_from_slice(b"ABC\0");
        };
        let mut f = Vec::new();
        block(&mut f, false);
        if version >= b'2' {
            block(&mut f, true);
            f.extend_from_slice(format!("\n{rule}\n").as_bytes());
        }
        f
    }

    #[test]
    fn a_zone_file_gives_the_offset_of_the_last_change() {
        let f = tzif(b'2', &[(1000, 1), (2000, 2)], &[-17_776, -18_000, -14_400], "<-04>4");
        assert_eq!(tzif_offset(&f, 999), Some(-17_776), "before the first change: the first type");
        assert_eq!(tzif_offset(&f, 1000), Some(-18_000));
        assert_eq!(tzif_offset(&f, 1999), Some(-18_000));
        assert_eq!(tzif_offset(&f, 2000), Some(-14_400), "after the last change: the rule");
        let f = tzif(b'2', &[(1000, 1)], &[0, 3600], "CET-1CEST,M3.5.0,M10.5.0/3");
        assert_eq!(tzif_offset(&f, utc(2026, 7, 1, 0, 0)), Some(7200), "the rule has daylight time");
        let f = tzif(b'2', &[(1000, 1)], &[0, 3600], "");
        assert_eq!(tzif_offset(&f, 5000), Some(3600), "no rule: the last type stays");
        let f = tzif(b'\0', &[(1000, 1)], &[0, 3600], "");
        assert_eq!(tzif_offset(&f, 999), Some(0), "version 1");
        assert_eq!(tzif_offset(&f, 1000), Some(3600));
        assert_eq!(tzif_offset(b"TZif2", 0), None, "cut short");
        assert_eq!(tzif_offset(b"not a zone file at all, but long enough to have a header", 0), None);
    }
}
