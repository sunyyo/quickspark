use crate::{
    model::{Action, Query, ResultItem},
    platform::windows::local_date,
};

use super::{Provider, ProviderContext, ProviderOutput, calculator::format_number};

const PROVIDER_ID: &str = "converter";

pub(crate) struct ConverterProvider;

impl ConverterProvider {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Provider for ConverterProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let converted = parse_unit_query(query.raw())
            .or_else(|| parse_date_query(query.raw()))
            .or_else(|| parse_base_query(query.raw()));
        let Some(converted) = converted else {
            if is_converter_query(query.raw()) {
                return ProviderOutput {
                    results: vec![ResultItem {
                        provider_id: PROVIDER_ID,
                        title: "换算格式无效；示例：%% 10 km to mi、date 2026-07-23 + 1d、hex 255"
                            .to_owned(),
                        score: 200_000,
                        actions: Vec::new(),
                    }],
                    exclusive: true,
                };
            }
            return ProviderOutput::empty();
        };
        ProviderOutput {
            results: vec![ResultItem {
                provider_id: PROVIDER_ID,
                title: format!("= {converted}"),
                score: 200_000,
                actions: vec![Action::CopyText {
                    text: converted.clone(),
                }],
            }],
            exclusive: true,
        }
    }
}

fn is_converter_query(input: &str) -> bool {
    let input = input.trim();
    input.starts_with("%%")
        || input.eq_ignore_ascii_case("date")
        || input.split_whitespace().next().is_some_and(|prefix| {
            ["date", "hex", "bin", "oct", "dec"]
                .iter()
                .any(|candidate| prefix.eq_ignore_ascii_case(candidate))
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dimension {
    Length,
    Mass,
    Duration,
    Speed,
    Data,
}

enum Unit {
    Linear {
        dimension: Dimension,
        factor: f64,
        label: &'static str,
    },
    Celsius,
    Fahrenheit,
    Kelvin,
}

fn unit(name: &str) -> Option<Unit> {
    match name.trim().to_ascii_lowercase().as_str() {
        "mm" => linear(Dimension::Length, 0.001, "mm"),
        "cm" => linear(Dimension::Length, 0.01, "cm"),
        "m" | "meter" | "meters" => linear(Dimension::Length, 1.0, "m"),
        "km" => linear(Dimension::Length, 1_000.0, "km"),
        "in" | "inch" | "inches" => linear(Dimension::Length, 0.0254, "in"),
        "ft" | "foot" | "feet" => linear(Dimension::Length, 0.3048, "ft"),
        "yd" | "yard" | "yards" => linear(Dimension::Length, 0.9144, "yd"),
        "mi" | "mile" | "miles" => linear(Dimension::Length, 1_609.344, "mi"),
        "mg" => linear(Dimension::Mass, 0.000_001, "mg"),
        "g" | "gram" | "grams" => linear(Dimension::Mass, 0.001, "g"),
        "kg" => linear(Dimension::Mass, 1.0, "kg"),
        "oz" => linear(Dimension::Mass, 0.028_349_523_125, "oz"),
        "lb" | "lbs" | "pound" | "pounds" => linear(Dimension::Mass, 0.453_592_37, "lb"),
        "ms" => linear(Dimension::Duration, 0.001, "ms"),
        "s" | "sec" | "second" | "seconds" => linear(Dimension::Duration, 1.0, "s"),
        "min" | "minute" | "minutes" => linear(Dimension::Duration, 60.0, "min"),
        "h" | "hr" | "hour" | "hours" => linear(Dimension::Duration, 3_600.0, "h"),
        "day" | "days" => linear(Dimension::Duration, 86_400.0, "day"),
        "m/s" | "mps" => linear(Dimension::Speed, 1.0, "m/s"),
        "km/h" | "kph" => linear(Dimension::Speed, 1.0 / 3.6, "km/h"),
        "mph" => linear(Dimension::Speed, 0.447_04, "mph"),
        "knot" | "knots" => linear(Dimension::Speed, 0.514_444_444_444, "knot"),
        "byte" | "bytes" => linear(Dimension::Data, 1.0, "byte"),
        "kb" => linear(Dimension::Data, 1_000.0, "KB"),
        "mb" => linear(Dimension::Data, 1_000_000.0, "MB"),
        "gb" => linear(Dimension::Data, 1_000_000_000.0, "GB"),
        "kib" => linear(Dimension::Data, 1_024.0, "KiB"),
        "mib" => linear(Dimension::Data, 1_048_576.0, "MiB"),
        "gib" => linear(Dimension::Data, 1_073_741_824.0, "GiB"),
        "c" | "℃" | "celsius" => Some(Unit::Celsius),
        "f" | "℉" | "fahrenheit" => Some(Unit::Fahrenheit),
        "k" | "kelvin" => Some(Unit::Kelvin),
        _ => None,
    }
}

fn linear(dimension: Dimension, factor: f64, label: &'static str) -> Option<Unit> {
    Some(Unit::Linear {
        dimension,
        factor,
        label,
    })
}

fn parse_unit_query(input: &str) -> Option<String> {
    let expression = input.trim().strip_prefix("%%")?.trim();
    let parts = expression.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 4 || !matches!(parts[2].to_ascii_lowercase().as_str(), "to" | "in") {
        return None;
    }
    let value = parts[0].parse::<f64>().ok()?;
    if !value.is_finite() {
        return None;
    }
    let source = unit(parts[1])?;
    let destination = unit(parts[3])?;
    let (converted, label) = convert(value, source, destination)?;
    if !converted.is_finite() {
        return None;
    }
    Some(format!("{} {label}", format_number(converted)))
}

fn convert(value: f64, source: Unit, destination: Unit) -> Option<(f64, &'static str)> {
    match (source, destination) {
        (
            Unit::Linear {
                dimension: left,
                factor: source_factor,
                ..
            },
            Unit::Linear {
                dimension: right,
                factor: destination_factor,
                label,
            },
        ) if left == right => Some((value * source_factor / destination_factor, label)),
        (source, destination) if is_temperature(&source) && is_temperature(&destination) => {
            let celsius = match source {
                Unit::Celsius => value,
                Unit::Fahrenheit => (value - 32.0) * 5.0 / 9.0,
                Unit::Kelvin => value - 273.15,
                Unit::Linear { .. } => unreachable!(),
            };
            let (converted, label) = match destination {
                Unit::Celsius => (celsius, "℃"),
                Unit::Fahrenheit => (celsius * 9.0 / 5.0 + 32.0, "℉"),
                Unit::Kelvin => (celsius + 273.15, "K"),
                Unit::Linear { .. } => unreachable!(),
            };
            Some((converted, label))
        }
        _ => None,
    }
}

fn is_temperature(unit: &Unit) -> bool {
    matches!(unit, Unit::Celsius | Unit::Fahrenheit | Unit::Kelvin)
}

fn parse_date_query(input: &str) -> Option<String> {
    let mut parts = input.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("date") {
        return None;
    }
    let first = parts.next();
    let date = match first {
        None => local_date(),
        Some(value) if value.eq_ignore_ascii_case("today") => local_date(),
        Some(value) => parse_date(value)?,
    };
    let remaining = parts.collect::<Vec<_>>();
    let result = match remaining.as_slice() {
        [] => date,
        [operator, amount] if matches!(*operator, "+" | "-") => {
            let days = parse_day_delta(amount)?;
            let signed_days = if *operator == "+" { days } else { -days };
            add_days(date, signed_days)?
        }
        _ => return None,
    };
    Some(format_date(result))
}

fn parse_day_delta(value: &str) -> Option<i64> {
    let lower = value.to_ascii_lowercase();
    if let Some(days) = lower.strip_suffix('d') {
        return days.parse().ok();
    }
    lower
        .strip_suffix('w')
        .and_then(|weeks| weeks.parse::<i64>().ok())
        .and_then(|weeks| weeks.checked_mul(7))
}

fn parse_date(value: &str) -> Option<(i32, u32, u32)> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !valid_date(year, month, day) {
        return None;
    }
    Some((year, month, day))
}

fn valid_date(year: i32, month: u32, day: u32) -> bool {
    if !(1..=9_999).contains(&year) || !(1..=12).contains(&month) {
        return false;
    }
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 31,
    };
    (1..=days).contains(&day)
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn add_days(date: (i32, u32, u32), days: i64) -> Option<(i32, u32, u32)> {
    let serial = days_from_civil(date.0, date.1, date.2).checked_add(days)?;
    let result = civil_from_days(serial);
    valid_date(result.0, result.1, result.2).then_some(result)
}

fn days_from_civil(mut year: i32, month: u32, day: u32) -> i64 {
    year -= i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let adjusted_month = month as i32 + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day as i32 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era as i64 * 146_097 + day_of_era as i64 - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

fn format_date(date: (i32, u32, u32)) -> String {
    format!("{:04}-{:02}-{:02}", date.0, date.1, date.2)
}

fn parse_base_query(input: &str) -> Option<String> {
    let mut parts = input.split_whitespace();
    let target = parts.next()?.to_ascii_lowercase();
    let value = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let default_radix = if target == "dec" { 16 } else { 10 };
    let (negative, magnitude) = parse_integer(value, default_radix)?;
    let sign = if negative { "-" } else { "" };
    match target.as_str() {
        "hex" => Some(format!("{sign}0x{magnitude:X}")),
        "bin" => Some(format!("{sign}0b{magnitude:b}")),
        "oct" => Some(format!("{sign}0o{magnitude:o}")),
        "dec" => Some(format!("{sign}{magnitude}")),
        _ => None,
    }
}

fn parse_integer(value: &str, default_radix: u32) -> Option<(bool, u128)> {
    let (negative, unsigned) = value
        .strip_prefix('-')
        .map_or((false, value), |rest| (true, rest));
    if unsigned.is_empty() {
        return None;
    }
    let (radix, digits) = if let Some(digits) = unsigned.strip_prefix("0x") {
        (16, digits)
    } else if let Some(digits) = unsigned.strip_prefix("0b") {
        (2, digits)
    } else if let Some(digits) = unsigned.strip_prefix("0o") {
        (8, digits)
    } else {
        (default_radix, unsigned)
    };
    u128::from_str_radix(digits, radix)
        .ok()
        .map(|magnitude| (negative, magnitude))
}

#[cfg(test)]
mod tests {
    use super::{add_days, parse_base_query, parse_date_query, parse_unit_query};

    #[test]
    fn converts_compatible_units_and_temperatures() {
        assert_eq!(parse_unit_query("%% 1 km to m").as_deref(), Some("1000 m"));
        assert_eq!(
            parse_unit_query("%% 100 f to c").as_deref(),
            Some("37.777777777778 ℃")
        );
        assert_eq!(parse_unit_query("%% 1 kg to m"), None);
        assert_eq!(parse_unit_query("%% 1e308 km to cm"), None);
    }

    #[test]
    fn performs_date_arithmetic_across_leap_days() {
        assert_eq!(
            parse_date_query("date 2024-02-28 + 1d").as_deref(),
            Some("2024-02-29")
        );
        assert_eq!(
            parse_date_query("date 2026-07-22 - 2w").as_deref(),
            Some("2026-07-08")
        );
        assert_eq!(add_days((2026, 12, 31), 1), Some((2027, 1, 1)));
        assert_eq!(parse_date_query("date 2025-02-29"), None);
    }

    #[test]
    fn converts_integer_bases() {
        assert_eq!(parse_base_query("hex 255").as_deref(), Some("0xFF"));
        assert_eq!(parse_base_query("bin 10").as_deref(), Some("0b1010"));
        assert_eq!(parse_base_query("oct 8").as_deref(), Some("0o10"));
        assert_eq!(parse_base_query("dec FF").as_deref(), Some("255"));
        assert_eq!(parse_base_query("dec 0b1010").as_deref(), Some("10"));
    }
}
