use chrono::{DateTime, Local};
use std::path::PathBuf;

pub fn build_session_relative_dir(primary_tag: &str, started_at: DateTime<Local>) -> PathBuf {
    let tag = sanitize_tag(primary_tag);
    let date_part = started_at.format("%d.%m.%Y").to_string();
    let time_part = started_at.format("%H-%M-%S").to_string();
    PathBuf::from(tag)
        .join(date_part)
        .join(format!("meeting_{}", time_part))
}

pub fn transcript_name(started_at: DateTime<Local>) -> String {
    format!("transcript_{}.txt", started_at.format("%d.%m.%Y"))
}

pub fn summary_name(started_at: DateTime<Local>) -> String {
    format!("summary_{}.txt", started_at.format("%d.%m.%Y"))
}

pub fn sanitize_tag(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c.to_ascii_lowercase());
        } else if c.is_whitespace() {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "general".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    #[test]
    fn builds_russian_date_layout() {
        let dt = FixedOffset::east_opt(3 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 3, 10, 15, 6, 7)
            .unwrap()
            .with_timezone(&Local);

        let p = build_session_relative_dir("Zoom Team", dt);
        assert!(p.ends_with("zoom_team/10.03.2026/meeting_15-06-07"));
    }

    #[test]
    fn builds_artifact_names_in_ru_date() {
        let dt = FixedOffset::east_opt(3 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 3, 10, 8, 0, 0)
            .unwrap()
            .with_timezone(&Local);

        assert_eq!(transcript_name(dt), "transcript_10.03.2026.txt");
        assert_eq!(summary_name(dt), "summary_10.03.2026.txt");
    }
}
