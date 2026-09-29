use aidoku::{
    Result,
    alloc::{String, Vec, string::ToString},
    imports::{
        html::{Document, Html},
        net::Request,
    },
};
use jiff::{SignedDuration, fmt::strtime, tz::TimeZone};

pub fn fetch_html(url: &str) -> Result<(Document, Option<String>)> {
    let raw = Request::get(url)?.string()?;
    let html = Html::parse(raw.clone())?;
    // Hack to approximate the current date from the cache update date
    let now_str = raw
        .rfind("<!-- WP Fastest Cache file was created")
        .and_then(|i| raw[i..].split("on ").nth(1))
        .and_then(|s| s.split("-->").next())
        .map(|s| s.replace("@ ", "").trim().to_string());

    Ok((html, now_str))
}

pub fn parse_chapter_title(chapter_str: Option<String>) -> Result<(Option<f32>, Option<String>)> {
    if let Some(ref s) = chapter_str {
        // Expect format "Chapter X" or "Chapter X - Some title"
        let prefix = "Chapter ";
        if let Some(rest) = s.strip_prefix(prefix) {
            let parts: Vec<&str> = rest.splitn(2, " - ").collect();

            let chapter_number = parts
                .first()
                .and_then(|num_str| num_str.trim().parse::<f32>().ok());

            let title = if parts.len() > 1 {
                Some(parts[1].trim().to_string())
            } else {
                None
            };

            return Ok((chapter_number, title));
        }
    }

    Ok((None, None))
}

pub fn parse_date_to_timestamp(date_str: &str, now_str: Option<&str>) -> Option<i64> {
    // Try absolute date: "July 17, 2025"
    if let Ok(date) = strtime::parse("%B %d, %Y", date_str).and_then(|tm| tm.to_date()) {
        return Some(
            date.at(0, 0, 0, 0)
                .to_zoned(TimeZone::UTC)
                .ok()?
                .timestamp()
                .as_second(),
        );
    }

    // Try relative date: "5 minutes ago", "2 days ago"
    let lowered = date_str.to_ascii_lowercase();
    let now = strtime::parse("%B %d, %Y %I:%M %p", now_str?)
        .ok()?
        .to_datetime()
        .ok()?
        .to_zoned(TimeZone::UTC)
        .ok()?
        .timestamp();
    let parts: Vec<&str> = lowered.split_whitespace().collect();
    if parts.len() >= 2
        && let Ok(value) = parts[0].parse::<i64>()
    {
        let unit = parts[1];
        let seconds_per_unit = match unit {
            "minute" | "minutes" => 60,
            "hour" | "hours" => 3_600,
            "day" | "days" => 86_400,
            _ => return None,
        };
        let seconds = value.checked_mul(seconds_per_unit)?;
        return Some(
            now.checked_sub(SignedDuration::from_secs(seconds))
                .ok()?
                .as_second(),
        );
    }

    None
}

#[cfg(test)]
mod test {
    use crate::helpers::{parse_chapter_title, parse_date_to_timestamp};
    use aidoku_test::aidoku_test;

    #[aidoku_test]
    fn parse_chapter_title_test() {
        let (chapter_number, title) = parse_chapter_title(Some("Chapter 1 - The Beginning".into()))
            .expect("parse_chapter_title failed");
        assert_eq!(chapter_number, Some(1.0));
        assert_eq!(title, Some("The Beginning".into()));

        let (chapter_number, title) =
            parse_chapter_title(Some("Chapter 2".into())).expect("parse_chapter_title failed");
        assert_eq!(chapter_number, Some(2.0));
        assert!(title.is_none());
    }

    #[aidoku_test]
    fn parse_date_to_timestamp_test() {
        let absolute_date_str = parse_date_to_timestamp("July 17, 2025", None);
        assert_eq!(absolute_date_str.unwrap(), 1752710400);

        let relative_date_str =
            parse_date_to_timestamp("5 minutes ago", Some("July 17, 2025 12:00 pm"));
        assert_eq!(relative_date_str.unwrap(), 1752753300);
    }
}
