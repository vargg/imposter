//! Generated values: `{{$uuid}}`, `{{$timestamp -1d}}`, `{{$randomInt 1 100}}` and friends.
//!
//! A placeholder whose name starts with `$` and names a function here is generated when a
//! request is sent, once per distinct expression: `{{$uuid}}` in the URL and the body is the
//! same value in one send, `{{$uuid a}}` and `{{$uuid b}}` are two. Words after a function's
//! own arguments are such labels. A variable the user defined with the same name wins.
//! Postman's names (`$guid`, `$randomUUID`, `$timestamp`, `$isoTimestamp`, `$randomInt`) work.
//!
//! This crate reads no clock and no randomness: the caller passes an [`Entropy`].

use std::fmt::Write as _;

use crate::execution::Timestamp;
use crate::model::{RequestTemplate, Variables};
use crate::variables::{ResolveError, placeholders, resolve_template};

/// The time and randomness generated values come from.
pub trait Entropy {
    /// Milliseconds since the Unix epoch.
    fn now(&self) -> Timestamp;
    /// The local time zone's offset from UTC at `now`, in seconds; `$date` uses it.
    fn utc_offset(&self) -> i32;
    fn random(&mut self) -> u64;
}

/// Every function, with what it makes; the editor suggests these after `{{$`.
pub const FUNCTIONS: [(&str, &str); 9] = [
    ("$uuid", "A random UUID (version 4)"),
    ("$guid", "A random UUID (Postman's name)"),
    ("$randomUUID", "A random UUID (Postman's name)"),
    (
        "$timestamp",
        "Unix time in seconds; add an offset like -1d or +2h",
    ),
    (
        "$timestampMs",
        "Unix time in milliseconds; add an offset like -5m",
    ),
    (
        "$isoTimestamp",
        "UTC time like 2026-10-31T09:30:00.000Z; add an offset like +1w",
    ),
    (
        "$date",
        "Local date or time, %Y-%m-%d by default; give a format and an offset",
    ),
    (
        "$randomInt",
        "A random integer, 0 to 1000 or between the two given",
    ),
    (
        "$randomString",
        "Random letters and digits, 16 or the given length",
    ),
];

const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const STRING_LIMIT: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Function {
    Uuid,
    Seconds,
    Millis,
    Iso,
    Date,
    RandomInt,
    RandomString,
}

/// Whether `name` is meant as a generated value (starts with `$`), whatever its arguments.
pub fn is_generated(name: &str) -> bool {
    name.starts_with('$')
}

/// What `name` would generate, or why it cannot: an unknown function or bad arguments.
/// `None` for names that are not generated values.
pub fn describe(name: &str) -> Option<Result<&'static str, String>> {
    if !is_generated(name) {
        return None;
    }
    let head = name.split_whitespace().next().unwrap_or_default();
    Some(
        match FUNCTIONS.iter().find(|(function, _)| *function == head) {
            None => Err(format!("{head} is not a generated value")),
            Some((_, description)) => parse(name).map(|_| *description),
        },
    )
}

struct Expression<'a> {
    function: Function,
    /// Milliseconds to add to now.
    offset: i64,
    format: &'a str,
    range: (i64, i64),
    length: usize,
}

/// What a word means where an offset may come.
enum Offset {
    /// Not shaped like an offset: a label.
    None,
    /// Milliseconds.
    Fits(i64),
    TooLarge,
}

/// `[+-]N` followed by `s`, `m`, `h`, `d` or `w`, in milliseconds.
fn offset(word: &str) -> Offset {
    let (sign, rest) = match word.as_bytes().first() {
        Some(b'-') => (-1, &word[1..]),
        Some(b'+') => (1, &word[1..]),
        _ => (1, word),
    };
    let unit = match rest.chars().last() {
        Some('s') => 1_000,
        Some('m') => 60_000,
        Some('h') => 3_600_000,
        Some('d') => 86_400_000,
        Some('w') => 604_800_000,
        _ => return Offset::None,
    };
    let digits = &rest[..rest.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Offset::None;
    }
    digits
        .parse::<i64>()
        .ok()
        .and_then(|amount| amount.checked_mul(unit)?.checked_mul(sign))
        .map_or(Offset::TooLarge, Offset::Fits)
}

fn parse(name: &str) -> Result<Expression<'_>, String> {
    let mut words = name.split_whitespace();
    let head = words.next().unwrap_or_default();
    let function = match head {
        "$uuid" | "$guid" | "$randomUUID" => Function::Uuid,
        "$timestamp" => Function::Seconds,
        "$timestampMs" => Function::Millis,
        "$isoTimestamp" => Function::Iso,
        "$date" => Function::Date,
        "$randomInt" => Function::RandomInt,
        "$randomString" => Function::RandomString,
        _ => return Err(format!("{head} is not a generated value")),
    };
    let mut words = words.peekable();
    let mut expression = Expression {
        function,
        offset: 0,
        format: "%Y-%m-%d",
        range: (0, 1000),
        length: 16,
    };
    match function {
        Function::Uuid => {}
        Function::Seconds | Function::Millis | Function::Iso | Function::Date => {
            if function == Function::Date
                && let Some(format) = words.next_if(|w| w.contains('%'))
            {
                expression.format = format;
            }
            match words.peek().map_or(Offset::None, |w| offset(w)) {
                Offset::Fits(ms) => expression.offset = ms,
                Offset::TooLarge => return Err("That offset is out of range".into()),
                Offset::None => {}
            }
        }
        Function::RandomInt => {
            let numbers: Vec<Option<i64>> = words.clone().take(2).map(|w| w.parse().ok()).collect();
            match numbers.as_slice() {
                [Some(min), Some(max)] if min <= max => expression.range = (*min, *max),
                [Some(_), Some(_)] => return Err("$randomInt needs min ≤ max".into()),
                [Some(_), ..] => return Err("$randomInt takes two numbers: min and max".into()),
                _ => {}
            }
        }
        Function::RandomString => {
            if let Some(word) = words.peek()
                && let Ok(length) = word.parse::<usize>()
            {
                if !(1..=STRING_LIMIT).contains(&length) {
                    return Err(format!("$randomString length must be 1 to {STRING_LIMIT}"));
                }
                expression.length = length;
            }
        }
    }
    Ok(expression)
}

/// An unbiased draw from `0..span` (Lemire's multiply-shift, rejecting the short tail).
fn below(entropy: &mut impl Entropy, span: u128) -> u128 {
    let span64 = u64::try_from(span).unwrap_or(u64::MAX);
    if span > u128::from(u64::MAX) {
        // Only for the full i64 range, where every u64 is a valid draw.
        return u128::from(entropy.random());
    }
    let threshold = span64.wrapping_neg() % span64;
    loop {
        let product = u128::from(entropy.random()) * span;
        #[allow(clippy::cast_possible_truncation)] // the low 64 bits, by definition
        if (product as u64) >= threshold {
            return product >> 64;
        }
    }
}

/// Generate the value of `name`, or say why it cannot be.
pub fn generate(name: &str, entropy: &mut impl Entropy) -> Result<String, String> {
    let expression = parse(name)?;
    let at = |offset: i64| {
        entropy
            .now()
            .0
            .checked_add(offset)
            .ok_or_else(|| "That offset is out of range".to_owned())
    };
    Ok(match expression.function {
        Function::Uuid => {
            let mut bytes = [0_u8; 16];
            bytes[..8].copy_from_slice(&entropy.random().to_be_bytes());
            bytes[8..].copy_from_slice(&entropy.random().to_be_bytes());
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            let hex = bytes.iter().fold(String::with_capacity(32), |mut hex, b| {
                let _ = write!(hex, "{b:02x}");
                hex
            });
            format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            )
        }
        Function::Seconds => at(expression.offset)?.div_euclid(1000).to_string(),
        Function::Millis => at(expression.offset)?.to_string(),
        Function::Iso => Timestamp(at(expression.offset)?).to_rfc3339(),
        Function::Date => {
            let millis = at(expression.offset)?;
            let zone = jiff::tz::Offset::from_seconds(entropy.utc_offset())
                .map_err(|error| error.to_string())?;
            let time = jiff::Timestamp::from_millisecond(millis)
                .map_err(|error| error.to_string())?
                .to_zoned(jiff::tz::TimeZone::fixed(zone));
            jiff::fmt::strtime::format(expression.format, &time)
                .map_err(|error| format!("Bad $date format: {error}"))?
        }
        Function::RandomInt => {
            let (min, max) = expression.range;
            let span = u128::try_from(i128::from(max) - i128::from(min) + 1).unwrap_or(1);
            let drawn = i128::try_from(below(entropy, span)).unwrap_or(0);
            i64::try_from(i128::from(min) + drawn)
                .unwrap_or(max)
                .to_string()
        }
        Function::RandomString => (0..expression.length)
            .map(|_| {
                let index =
                    usize::try_from(below(entropy, ALPHANUMERIC.len() as u128)).unwrap_or(0);
                char::from(ALPHANUMERIC[index])
            })
            .collect(),
    })
}

/// Add a value for every generated placeholder `template` uses (where it reaches the wire,
/// and in its checks' expected values) that `values` does not already define. Expressions
/// that cannot be generated are left out, so resolving reports them as unresolved.
pub fn fill(template: &RequestTemplate, values: &mut Variables, entropy: &mut impl Entropy) {
    let mut names: Vec<String> = match resolve_template(template, values) {
        Err(ResolveError::Unresolved(missing)) => missing.into_iter().collect(),
        _ => Vec::new(),
    };
    for check in &template.checks {
        names.extend(
            placeholders(&check.expected)
                .into_iter()
                .map(|p| p.name.to_owned()),
        );
    }
    for name in names {
        if is_generated(&name)
            && !values.contains_key(&name)
            && let Ok(value) = generate(&name, entropy)
        {
            values.insert(name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{Check, Op};
    use crate::select::Selector;

    /// 2026-10-31T09:30:00.250Z, two hours east of UTC, counting random numbers.
    struct Fixed {
        next: u64,
    }

    impl Entropy for Fixed {
        fn now(&self) -> Timestamp {
            Timestamp(1_793_439_000_250)
        }
        fn utc_offset(&self) -> i32 {
            7_200
        }
        fn random(&mut self) -> u64 {
            self.next = self.next.wrapping_add(0x9e37_79b9_7f4a_7c15);
            self.next
        }
    }

    fn run(name: &str) -> Result<String, String> {
        generate(name, &mut Fixed { next: 0 })
    }

    #[test]
    fn times_and_dates_with_offsets() {
        assert_eq!(run("$timestamp").unwrap(), "1793439000");
        assert_eq!(run("$timestamp -1d").unwrap(), "1793352600");
        assert_eq!(run("$timestampMs +1s").unwrap(), "1793439001250");
        assert_eq!(run("$isoTimestamp").unwrap(), "2026-10-31T09:30:00.250Z");
        assert_eq!(
            run("$isoTimestamp +2w").unwrap(),
            "2026-11-14T09:30:00.250Z"
        );
        assert_eq!(
            run("$isoTimestamp -90m").unwrap(),
            "2026-10-31T08:00:00.250Z"
        );
        assert_eq!(run("$date").unwrap(), "2026-10-31", "local time");
        assert_eq!(run("$date %H:%M").unwrap(), "11:30");
        assert_eq!(
            run("$date %Y-%m-%dT%H:%M -12h").unwrap(),
            "2026-10-30T23:30"
        );
        assert_eq!(run("$date +1d").unwrap(), "2026-11-01");
        assert_eq!(
            run("$timestamp a").unwrap(),
            "1793439000",
            "a word that is not an offset is a label"
        );
        assert!(
            run("$date %Y%")
                .unwrap_err()
                .starts_with("Bad $date format")
        );
        assert!(run("$timestamp +999999999999w").is_err());
    }

    #[test]
    fn uuids_are_version_4() {
        let uuid = run("$uuid").unwrap();
        assert_eq!(uuid.len(), 36);
        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
        assert_eq!(run("$guid").unwrap(), uuid, "same entropy, same value");
        assert_eq!(run("$randomUUID label").unwrap(), uuid);
    }

    #[test]
    fn random_numbers_and_strings_stay_in_bounds() {
        let mut entropy = Fixed { next: 0 };
        for _ in 0..500 {
            let n: i64 = generate("$randomInt 5 7", &mut entropy)
                .unwrap()
                .parse()
                .unwrap();
            assert!((5..=7).contains(&n));
            let d: i64 = generate("$randomInt", &mut entropy)
                .unwrap()
                .parse()
                .unwrap();
            assert!((0..=1000).contains(&d));
        }
        assert_eq!(run("$randomInt -3 -3").unwrap(), "-3");
        let full = format!("$randomInt {} {}", i64::MIN, i64::MAX);
        assert!(run(&full).unwrap().parse::<i64>().is_ok());
        assert!(run("$randomInt 9 1").unwrap_err().contains("min ≤ max"));
        assert!(run("$randomInt 9").is_err());
        assert!(run("$randomInt 9 x").is_err());
        assert!(run("$randomInt x 9").is_ok(), "x is a label");

        let text = run("$randomString").unwrap();
        assert_eq!(text.len(), 16);
        assert!(text.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_eq!(run("$randomString 3 a").unwrap().len(), 3);
        assert!(run("$randomString 0").is_err());
        assert!(run("$randomString 1025").is_err());
    }

    #[test]
    fn unknown_names_explain_themselves() {
        assert_eq!(describe("token"), None);
        assert_eq!(describe("$uuid"), Some(Ok("A random UUID (version 4)")));
        assert_eq!(
            describe("$randomFirstName"),
            Some(Err("$randomFirstName is not a generated value".into()))
        );
        assert_eq!(
            describe("$randomInt 3 1"),
            Some(Err("$randomInt needs min ≤ max".into()))
        );
        assert!(FUNCTIONS.iter().all(|(name, _)| describe(name)
            == Some(Ok(FUNCTIONS.iter().find(|(n, _)| n == name).unwrap().1))));
    }

    #[test]
    fn filling_generates_each_expression_once_and_keeps_defined_names() {
        let template = RequestTemplate {
            url: "http://x/{{$uuid}}/{{$uuid}}?b={{$uuid b}}&n={{$randomInt 1 9}}".into(),
            body: b"{\"id\": \"{{$uuid}}\", \"who\": \"{{$guid}}\", \"x\": \"{{$nope}}\"}".to_vec(),
            body_type: crate::model::BodyType::Json,
            checks: vec![Check::new(
                Selector::Json("$.at".into()),
                Op::Equals,
                "{{$timestamp}}",
            )],
            ..RequestTemplate::default()
        };
        let mut values: Variables = [("$guid".to_owned(), "mine".to_owned())]
            .into_iter()
            .collect();
        fill(&template, &mut values, &mut Fixed { next: 0 });
        assert_eq!(values["$guid"], "mine", "a defined name wins");
        assert!(values.contains_key("$uuid") && values.contains_key("$uuid b"));
        assert_ne!(values["$uuid"], values["$uuid b"]);
        assert_eq!(
            values["$timestamp"], "1793439000",
            "checks' placeholders too"
        );
        assert!(!values.contains_key("$nope"));
        let error = resolve_template(&template, &values)
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Unresolved variables: $nope");

        let resolved = resolve_template(
            &RequestTemplate {
                body: Vec::new(),
                ..template.clone()
            },
            &values,
        )
        .unwrap();
        let uuid = &values["$uuid"];
        assert!(
            resolved
                .url
                .starts_with(&format!("http://x/{uuid}/{uuid}?b="))
        );
    }
}
