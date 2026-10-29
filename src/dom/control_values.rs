//! HTML common microsyntaxes for form control values (HTML §2.3.4 numbers, §2.3.5 dates and
//! times, §4.10.5.1.5 email addresses) and the input types' "convert a string to a number"
//! algorithms (HTML §4.10.5.1).
use crate::dom::state::is_html_whitespace;

const GREGORIAN_CYCLE: u32 = 400;
const DAYS_PER_CYCLE: f64 = 146_097.0;
const MONTHS_PER_YEAR: u8 = 12;
const HOURS_PER_DAY: u8 = 24;
const MINUTES_PER_HOUR: u8 = 60;
const SECONDS_PER_MINUTE: u8 = 60;
const MIN_YEAR_DIGITS: usize = 4;
const DECIMAL_RADIX: u8 = 10;
const DIGIT_PAIR_LENGTH: usize = 2;
const LEAP_PERIOD: u32 = 4;
const CENTURY: u32 = 100;
const DAYS_PER_WEEK: u32 = 7;
const WEDNESDAY: u32 = 3;
const THURSDAY: u32 = 4;
const REGULAR_WEEKS: u8 = 52;
const LONG_WEEKS: u8 = 53;
const EPOCH_YEAR: f64 = 1970.0;
/// ISO 8601 week one is the week containing 4 January.
const WEEK_ONE_DAY: u8 = 4;
/// 1970-01-01 was a Thursday: days since the epoch plus this offset count from a Monday.
const EPOCH_WEEKDAY_FROM_MONDAY: f64 = 3.0;
pub(crate) const MS_PER_SECOND: f64 = 1000.0;
pub(crate) const MS_PER_MINUTE: f64 = 60_000.0;
pub(crate) const MS_PER_DAY: f64 = 86_400_000.0;
pub(crate) const MS_PER_WEEK: f64 = 604_800_000.0;
const EMAIL_LABEL_LIMIT: usize = 63;
const EMAIL_ATEXT_SYMBOLS: &str = ".!#$%&'*+/=?^_`{|}~-";

fn two_digits(value: &str) -> Option<u8> {
    (value.len() == DIGIT_PAIR_LENGTH && value.bytes().all(|ch| ch.is_ascii_digit()))
        .then(|| (value.as_bytes()[0] - b'0') * DECIMAL_RADIX + value.as_bytes()[1] - b'0')
}

/// A valid year: four or more digits denoting a number greater than zero. Leap years repeat
/// every Gregorian cycle, so validity only needs the year modulo 400; the numeric value is
/// kept as a float for the epoch conversions.
#[derive(Clone, Copy)]
struct Year {
    cycle: u32,
    value: f64,
}
fn year(value: &str) -> Option<Year> {
    (value.len() >= MIN_YEAR_DIGITS
        && value.bytes().all(|ch| ch.is_ascii_digit())
        && value.bytes().any(|ch| ch != b'0'))
    .then(|| Year {
        cycle: value.bytes().fold(0, |year, digit| {
            (year * u32::from(DECIMAL_RADIX) + u32::from(digit - b'0')) % GREGORIAN_CYCLE
        }),
        value: value.parse().expect("ASCII digits parse as a float"),
    })
}
fn leap(cycle: u32) -> bool {
    cycle.is_multiple_of(LEAP_PERIOD)
        && (!cycle.is_multiple_of(CENTURY) || cycle.is_multiple_of(GREGORIAN_CYCLE))
}

/// Days from 1970-01-01 to the given proleptic Gregorian date.
fn days_since_epoch(year: Year, month: u8, day: u8) -> f64 {
    // Whole cycles keep the civil-date arithmetic within the cycle-relative year.
    let cycles = ((year.value - f64::from(year.cycle)) / f64::from(GREGORIAN_CYCLE)).round();
    cycles * DAYS_PER_CYCLE + civil_days(i64::from(year.cycle), month, day)
        - civil_days(EPOCH_YEAR as i64, 1, 1)
}
/// Days since 0000-03-01 of a proleptic Gregorian date.
fn civil_days(year: i64, month: u8, day: u8) -> f64 {
    const MARCH: i64 = 3;
    const DAYS_PER_FIVE_MONTHS: i64 = 153;
    const FIVE_MONTHS: i64 = 5;
    const DAYS_PER_YEAR: i64 = 365;
    let year = if i64::from(month) < MARCH {
        year - 1
    } else {
        year
    };
    let era = year.div_euclid(i64::from(GREGORIAN_CYCLE));
    let of_era = year.rem_euclid(i64::from(GREGORIAN_CYCLE));
    let month_from_march =
        (i64::from(month) + i64::from(MONTHS_PER_YEAR) - MARCH) % i64::from(MONTHS_PER_YEAR);
    let of_year = (DAYS_PER_FIVE_MONTHS * month_from_march + 2) / FIVE_MONTHS + i64::from(day) - 1;
    let of_cycle = of_era * DAYS_PER_YEAR + of_era / i64::from(LEAP_PERIOD)
        - of_era / i64::from(CENTURY)
        + of_year;
    (era * DAYS_PER_CYCLE as i64 + of_cycle) as f64
}

/// HTML §2.3.5.1: a valid month string, as its year and month.
fn month(value: &str) -> Option<(Year, u8)> {
    let (y, m) = value.split_once('-')?;
    let y = year(y)?;
    let m = two_digits(m)?;
    (m != 0 && m <= MONTHS_PER_YEAR).then_some((y, m))
}
/// HTML §2.3.5.2: a valid date string, as its year, month and day.
fn date(value: &str) -> Option<(Year, u8, u8)> {
    let (ym, day) = value.rsplit_once('-')?;
    let ((year, month), day) = (month(ym)?, two_digits(day)?);
    let days = match month {
        2 if leap(year.cycle) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (day != 0 && day <= days).then_some((year, month, day))
}
/// HTML §2.3.5.8: a valid week string, as its week-year and week.
fn week(value: &str) -> Option<(Year, u8)> {
    let (y, week) = value.split_once("-W")?;
    let (y, week) = (year(y)?, two_digits(week)?);
    let prior = (y.cycle + GREGORIAN_CYCLE - 1) % GREGORIAN_CYCLE;
    let jan_first = (prior + prior / LEAP_PERIOD - prior / CENTURY + 1) % DAYS_PER_WEEK;
    let last = if jan_first == THURSDAY || (jan_first == WEDNESDAY && leap(y.cycle)) {
        LONG_WEEKS
    } else {
        REGULAR_WEEKS
    };
    (week != 0 && week <= last).then_some((y, week))
}
/// HTML §2.3.5.4: a valid time string, as milliseconds since midnight.
fn time(value: &str) -> Option<f64> {
    let mut parts = value.split(':');
    let hour = parts.next().and_then(two_digits)?;
    let minute = parts.next().and_then(two_digits)?;
    if hour >= HOURS_PER_DAY || minute >= MINUTES_PER_HOUR {
        return None;
    }
    let mut ms =
        (f64::from(hour) * f64::from(MINUTES_PER_HOUR) + f64::from(minute)) * MS_PER_MINUTE;
    if let Some(seconds) = parts.next() {
        let (whole, fraction) = seconds
            .split_once('.')
            .map_or((seconds, None), |(s, f)| (s, Some(f)));
        let whole = two_digits(whole).filter(|s| *s < SECONDS_PER_MINUTE)?;
        if fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|ch| ch.is_ascii_digit())) {
            return None;
        }
        let fraction: f64 = fraction.map_or(0.0, |f| format!("0.{f}").parse().unwrap_or(0.0));
        ms += (f64::from(whole) + fraction) * MS_PER_SECOND;
    }
    parts.next().is_none().then_some(ms)
}
/// HTML §2.3.5.5: a valid local date and time string, split at its separator.
fn local_date_time(value: &str) -> Option<(&str, &str)> {
    let (date_part, time_part) = value.split_once(['T', ' '])?;
    (date(date_part).is_some() && time(time_part).is_some()).then_some((date_part, time_part))
}

pub(crate) fn valid_month(value: &str) -> bool {
    month(value).is_some()
}
pub(crate) fn valid_date(value: &str) -> bool {
    date(value).is_some()
}
pub(crate) fn valid_week(value: &str) -> bool {
    week(value).is_some()
}
pub(crate) fn valid_time(value: &str) -> bool {
    time(value).is_some()
}

/// HTML §2.3.5.5: the valid normalized local date and time string for a valid one: a `T`
/// separator and the shortest time representing the same instant.
pub(crate) fn normalized_local_date_time(value: &str) -> Option<String> {
    let (date_part, time_part) = local_date_time(value)?;
    let mut time_part = time_part.to_owned();
    if let Some(point) = time_part.find('.') {
        let trimmed = time_part.trim_end_matches('0').len();
        time_part.truncate(if trimmed == point + 1 { point } else { trimmed });
    }
    if let Some(seconds) = time_part.strip_suffix(":00") {
        time_part = seconds.to_owned();
    }
    Some(format!("{date_part}T{time_part}"))
}

/// Milliseconds from the epoch to midnight UTC of a valid date string.
pub(crate) fn date_number(value: &str) -> Option<f64> {
    let (year, month, day) = date(value)?;
    Some(days_since_epoch(year, month, day) * MS_PER_DAY)
}
/// Months between January 1970 and a valid month string.
pub(crate) fn month_number(value: &str) -> Option<f64> {
    let (year, month) = month(value)?;
    Some((year.value - EPOCH_YEAR) * f64::from(MONTHS_PER_YEAR) + f64::from(month) - 1.0)
}
/// Milliseconds from the epoch to midnight UTC of the Monday starting a valid week string.
pub(crate) fn week_number(value: &str) -> Option<f64> {
    let (year, week) = week(value)?;
    let fourth = days_since_epoch(year, 1, WEEK_ONE_DAY);
    let monday = fourth - (fourth + EPOCH_WEEKDAY_FROM_MONDAY).rem_euclid(f64::from(DAYS_PER_WEEK));
    Some((monday + f64::from(week - 1) * f64::from(DAYS_PER_WEEK)) * MS_PER_DAY)
}
/// Milliseconds since midnight of a valid time string.
pub(crate) fn time_number(value: &str) -> Option<f64> {
    time(value)
}
/// Milliseconds from the epoch to a valid local date and time string, read as UTC.
pub(crate) fn local_date_time_number(value: &str) -> Option<f64> {
    let (date_part, time_part) = local_date_time(value)?;
    Some(date_number(date_part)? + time(time_part)?)
}

/// HTML §2.3.4.3: a valid floating-point number that converts to a finite number.
pub(crate) fn valid_float(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let (significand, exponent) = unsigned
        .split_once(['e', 'E'])
        .map_or((unsigned, None), |(s, e)| (s, Some(e)));
    if exponent.is_some_and(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        e.is_empty() || !e.bytes().all(|ch| ch.is_ascii_digit())
    }) {
        return false;
    }
    let digits = if let Some((integer, fraction)) = significand.split_once('.') {
        integer.bytes().all(|ch| ch.is_ascii_digit())
            && !fraction.is_empty()
            && fraction.bytes().all(|ch| ch.is_ascii_digit())
    } else {
        !significand.is_empty() && significand.bytes().all(|ch| ch.is_ascii_digit())
    };
    digits && parse_float(value).is_some()
}

/// HTML §2.3.4.3 rules for parsing floating-point number values: the longest numeric prefix
/// after white space, rounded to the nearest finite double.
pub(crate) fn parse_float(input: &str) -> Option<f64> {
    let rest = input.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let (negative, rest) = match rest.as_bytes().first()? {
        b'-' => (true, &rest[1..]),
        b'+' => (false, &rest[1..]),
        _ => (false, rest),
    };
    let integer_length = rest.bytes().take_while(u8::is_ascii_digit).count();
    let (integer, mut rest) = rest.split_at(integer_length);
    let mut fraction = "";
    if let Some(after) = rest.strip_prefix('.') {
        let length = after.bytes().take_while(u8::is_ascii_digit).count();
        if integer.is_empty() && length == 0 {
            return None;
        }
        fraction = &after[..length];
        rest = &after[length..];
    } else if integer.is_empty() {
        return None;
    }
    let mut exponent = String::new();
    if let Some(after) = rest.strip_prefix(['e', 'E']) {
        let (sign, digits) = match after.as_bytes().first() {
            Some(b'-') => ("-", &after[1..]),
            Some(b'+') => ("", &after[1..]),
            _ => ("", after),
        };
        let length = digits.bytes().take_while(u8::is_ascii_digit).count();
        if length > 0 {
            exponent = format!("e{sign}{}", &digits[..length]);
        }
    }
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = if fraction.is_empty() { "0" } else { fraction };
    let value: f64 = format!("{integer}.{fraction}{exponent}")
        .parse()
        .expect("digits form a float literal");
    if !value.is_finite() {
        return None;
    }
    // The rounding set excludes negative zero.
    Some(if negative && value != 0.0 {
        -value
    } else {
        value
    })
}

/// HTML §4.10.5.1.5: a valid email address.
pub(crate) fn valid_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || EMAIL_ATEXT_SYMBOLS.contains(ch))
        && domain.split('.').all(|label| {
            let bytes = label.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= EMAIL_LABEL_LIMIT
                && bytes[0].is_ascii_alphanumeric()
                && bytes[bytes.len() - 1].is_ascii_alphanumeric()
                && bytes
                    .iter()
                    .all(|ch| ch.is_ascii_alphanumeric() || *ch == b'-')
        })
}

/// HTML §4.10.5.1.5: a valid email address list's comma-separated tokens.
pub(crate) fn valid_email_list(value: &str) -> bool {
    value.split(',').all(valid_email)
}

/// HTML §4.10.5.1.4: a valid absolute URL, which the URL Standard parses without a
/// validation error.
pub(crate) fn valid_absolute_url(value: &str) -> bool {
    let violation = std::cell::Cell::new(false);
    url::Url::options()
        .syntax_violation_callback(Some(&|_| violation.set(true)))
        .parse(value)
        .is_ok()
        && !violation.get()
}

/// The URL and single-address email sanitization: no line breaks, then no leading or trailing
/// ASCII white space.
pub(crate) fn trimmed_line(value: &str) -> String {
    stripped_newlines(value)
        .trim_matches(is_html_whitespace)
        .to_owned()
}

/// "Strip newlines from the value".
pub(crate) fn stripped_newlines(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, '\r' | '\n'))
        .collect()
}
