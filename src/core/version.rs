use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionKind {
    Semver,
    Calver,
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub raw: String,
    pub kind: VersionKind,
    pub semver: Option<semver::Version>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionReq {
    pub raw: String,
    pub semver: Option<semver::VersionReq>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpMode {
    Semver,
    Calver,
    TinyInc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpLevel {
    Major,
    Minor,
    Patch,
}

#[derive(Debug, Error)]
pub enum VersionError {
    #[error("invalid semver version '{0}'")]
    InvalidSemver(String),
    #[error("invalid prerelease tag '{0}'")]
    InvalidPrerelease(String),
    #[error("missing numeric segment to bump")]
    NoNumericSegment,
}

pub type VersionResult<T> = std::result::Result<T, VersionError>;

impl Version {
    pub fn new(raw: impl Into<String>, kind: VersionKind) -> Self {
        let raw = raw.into();
        let semver = match kind {
            VersionKind::Semver => semver::Version::parse(&raw).ok(),
            _ => None,
        };
        Self { raw, kind, semver }
    }
}

impl VersionReq {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let semver = semver::VersionReq::parse(&raw).ok();
        Self { raw, semver }
    }
}

pub fn parse_bump_level(input: &str) -> Option<BumpLevel> {
    match input.to_ascii_lowercase().as_str() {
        "major" => Some(BumpLevel::Major),
        "minor" => Some(BumpLevel::Minor),
        "patch" => Some(BumpLevel::Patch),
        _ => None,
    }
}

pub fn parse_bump_mode(input: &str) -> Option<BumpMode> {
    match input.to_ascii_lowercase().as_str() {
        "semver" => Some(BumpMode::Semver),
        "calver" => Some(BumpMode::Calver),
        "tinyinc" => Some(BumpMode::TinyInc),
        _ => None,
    }
}

pub fn parse_version_kind(input: &str) -> Option<VersionKind> {
    match input.to_ascii_lowercase().as_str() {
        "semver" => Some(VersionKind::Semver),
        "calver" => Some(VersionKind::Calver),
        "raw" | "none" => Some(VersionKind::Raw),
        _ => None,
    }
}

pub fn bump_version(
    current: &Version,
    mode: BumpMode,
    level: Option<BumpLevel>,
    calver_format: Option<&str>,
    pre: Option<&str>,
) -> VersionResult<Version> {
    match mode {
        BumpMode::Semver => bump_semver(current, level.unwrap_or(BumpLevel::Patch), pre),
        BumpMode::Calver => {
            let format = calver_format.unwrap_or("YYYY.0M.MICRO");
            let raw = bump_calver(&current.raw, format)?;
            Ok(Version::new(raw, VersionKind::Calver))
        }
        BumpMode::TinyInc => {
            let raw = bump_rightmost_numeric(&current.raw)?;
            Ok(Version::new(raw, current.kind.clone()))
        }
    }
}

pub fn bump_rightmost_numeric(raw: &str) -> VersionResult<String> {
    let bytes = raw.as_bytes();
    let mut end = None;
    for idx in (0..bytes.len()).rev() {
        if bytes[idx].is_ascii_digit() {
            end = Some(idx);
            break;
        }
    }
    let end = end.ok_or(VersionError::NoNumericSegment)?;
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    let number_str = &raw[start..=end];
    let number = number_str
        .parse::<u64>()
        .map_err(|_| VersionError::NoNumericSegment)?;
    let next = number + 1;
    let replacement = if number_str.starts_with('0') && number_str.len() > 1 {
        format!("{:0width$}", next, width = number_str.len())
    } else {
        next.to_string()
    };
    let mut out = String::new();
    out.push_str(&raw[..start]);
    out.push_str(&replacement);
    out.push_str(&raw[end + 1..]);
    Ok(out)
}

fn bump_semver(current: &Version, level: BumpLevel, pre: Option<&str>) -> VersionResult<Version> {
    let mut version = current
        .semver
        .clone()
        .or_else(|| semver::Version::parse(&current.raw).ok())
        .ok_or_else(|| VersionError::InvalidSemver(current.raw.clone()))?;
    match level {
        BumpLevel::Major => {
            version.major += 1;
            version.minor = 0;
            version.patch = 0;
        }
        BumpLevel::Minor => {
            version.minor += 1;
            version.patch = 0;
        }
        BumpLevel::Patch => {
            version.patch += 1;
        }
    }
    let prerelease = if let Some(tag) = pre {
        semver::Prerelease::new(tag)
            .map_err(|_| VersionError::InvalidPrerelease(tag.to_string()))?
    } else {
        semver::Prerelease::EMPTY
    };
    version.pre = prerelease;
    version.build = semver::BuildMetadata::EMPTY;
    let raw = version.to_string();
    Ok(Version {
        raw,
        kind: VersionKind::Semver,
        semver: Some(version),
    })
}

fn bump_calver(current_raw: &str, format: &str) -> VersionResult<String> {
    let date = current_date();
    let template = apply_calver_format(format, date);
    if let Some(idx) = template.find("{MICRO}") {
        let prefix = &template[..idx];
        let suffix = &template[idx + "{MICRO}".len()..];
        let mut old_value: Option<String> = None;
        if current_raw.starts_with(prefix) && current_raw.ends_with(suffix) {
            let start = prefix.len();
            let end = current_raw.len().saturating_sub(suffix.len());
            if start <= end && end <= current_raw.len() {
                let middle = &current_raw[start..end];
                if !middle.is_empty() && middle.chars().all(|ch| ch.is_ascii_digit()) {
                    old_value = Some(middle.to_string());
                }
            }
        }
        let next = match old_value.as_ref().and_then(|v| v.parse::<u64>().ok()) {
            Some(value) => value + 1,
            None => 1,
        };
        let replacement = match old_value {
            Some(value) if value.starts_with('0') && value.len() > 1 => {
                format!("{:0width$}", next, width = value.len())
            }
            _ => next.to_string(),
        };
        return Ok(format!("{prefix}{replacement}{suffix}"));
    }

    bump_rightmost_numeric(&template)
}

#[derive(Clone, Copy)]
struct CalverDate {
    year: i32,
    month: u32,
    day: u32,
}

fn current_date() -> CalverDate {
    let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i64,
        Err(_) => 0,
    };
    let days = secs / 86_400;
    let (year, month, day) = civil_from_days(days);
    CalverDate { year, month, day }
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

fn apply_calver_format(format: &str, date: CalverDate) -> String {
    let mut out = String::new();
    let mut idx = 0;
    while idx < format.len() {
        let remaining = &format[idx..];
        if remaining.starts_with("YYYY") {
            out.push_str(&format!("{:04}", date.year));
            idx += 4;
            continue;
        }
        if remaining.starts_with("YY") {
            out.push_str(&format!("{:02}", (date.year % 100)));
            idx += 2;
            continue;
        }
        if remaining.starts_with("0M") || remaining.starts_with("MM") {
            out.push_str(&format!("{:02}", date.month));
            idx += 2;
            continue;
        }
        if remaining.starts_with("0D") || remaining.starts_with("DD") {
            out.push_str(&format!("{:02}", date.day));
            idx += 2;
            continue;
        }
        if remaining.starts_with("MICRO") {
            out.push_str("{MICRO}");
            idx += 5;
            continue;
        }
        if let Some(ch) = remaining.chars().next() {
            out.push(ch);
            idx += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::core::version::{
        apply_calver_format, bump_calver, bump_rightmost_numeric, bump_version, current_date,
        parse_bump_level, parse_bump_mode, parse_version_kind, BumpLevel, BumpMode, Version,
        VersionKind,
    };

    #[test]
    fn semver_bump_patch_with_prerelease() {
        let current = Version::new("1.2.3", VersionKind::Semver);
        let bumped = bump_version(
            &current,
            BumpMode::Semver,
            Some(BumpLevel::Patch),
            None,
            Some("rc.1"),
        )
        .expect("bump semver");
        assert_eq!(bumped.raw, "1.2.4-rc.1");
        assert!(bumped.semver.is_some());
    }

    #[test]
    fn tinyinc_bumps_rightmost_numeric_with_zero_padding() {
        let bumped = bump_rightmost_numeric("2026.02.009").expect("bump");
        assert_eq!(bumped, "2026.02.010");
    }

    #[test]
    fn tinyinc_errors_without_numeric_segment() {
        let err = bump_rightmost_numeric("release").expect_err("expected no numeric segment");
        assert_eq!(err.to_string(), "missing numeric segment to bump");
    }

    #[test]
    fn calver_micro_increments_when_format_matches() {
        let date = current_date();
        let template = apply_calver_format("YYYY.0M.{MICRO}", date);
        let current = template.replace("{MICRO}", "009");
        let bumped = bump_calver(&current, "YYYY.0M.{MICRO}").expect("bump calver");
        assert_eq!(bumped, template.replace("{MICRO}", "010"));
    }

    #[test]
    fn parser_helpers_accept_expected_values() {
        assert_eq!(parse_bump_level("major"), Some(BumpLevel::Major));
        assert_eq!(parse_bump_mode("tinyinc"), Some(BumpMode::TinyInc));
        assert_eq!(parse_version_kind("none"), Some(VersionKind::Raw));
    }
}
