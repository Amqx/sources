use aidoku::{
	MangaStatus, Viewer,
	alloc::{String, format},
	imports::std::{current_date, parse_date},
};

pub const BASE_URL: &str = "https://projectsuki.com";

pub fn valid_id(value: &str) -> bool {
	!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn absolute_url(url: &str) -> String {
	if url.starts_with("http://") || url.starts_with("https://") {
		url.into()
	} else if url.starts_with('/') {
		format!("{BASE_URL}{url}")
	} else {
		format!("{BASE_URL}/{url}")
	}
}

pub fn manga_url(key: &str) -> String {
	format!("{BASE_URL}/book/{key}")
}

pub fn thumbnail_url(key: &str) -> String {
	format!("{BASE_URL}/images/gallery/{key}/thumb")
}

fn path_segments(url: &str) -> impl Iterator<Item = &str> {
	let relative = url.strip_prefix(BASE_URL).unwrap_or(url);
	relative
		.split(['?', '#'])
		.next()
		.unwrap_or(relative)
		.split('/')
		.filter(|segment| !segment.is_empty())
}

pub fn manga_key_from_url(url: &str) -> Option<String> {
	let mut segments = path_segments(url);
	match (segments.next(), segments.next(), segments.next()) {
		(Some("book"), Some(key), None) if valid_id(key) => Some(key.into()),
		_ => None,
	}
}

pub fn chapter_keys_from_url(url: &str) -> Option<(String, String)> {
	let mut segments = path_segments(url);
	match (segments.next(), segments.next(), segments.next()) {
		(Some("read"), Some(manga_key), Some(chapter_id))
			if valid_id(manga_key) && valid_id(chapter_id) =>
		{
			Some((manga_key.into(), chapter_id.into()))
		}
		_ => None,
	}
}

pub fn chapter_key(manga_key: &str, chapter_id: &str) -> String {
	format!("{manga_key}/{chapter_id}")
}

pub fn split_chapter_key(key: &str) -> Option<(&str, &str)> {
	let (manga_key, chapter_id) = key.split_once('/')?;
	(valid_id(manga_key) && valid_id(chapter_id)).then_some((manga_key, chapter_id))
}

pub fn chapter_number(title: &str) -> Option<f32> {
	let start = title.find(|character: char| character.is_ascii_digit())?;
	let number = &title[start..];
	let end = number
		.find(|character: char| !character.is_ascii_digit() && character != '.')
		.unwrap_or(number.len());
	number[..end].trim_end_matches('.').parse().ok()
}

pub fn manga_status(status: &str) -> MangaStatus {
	match status.trim().to_ascii_lowercase().as_str() {
		"ongoing" => MangaStatus::Ongoing,
		"completed" => MangaStatus::Completed,
		"hiatus" => MangaStatus::Hiatus,
		"cancelled" | "canceled" => MangaStatus::Cancelled,
		_ => MangaStatus::Unknown,
	}
}

pub fn origin_format(origin: &str) -> Option<&'static str> {
	match origin.trim().to_ascii_lowercase().as_str() {
		"korea" | "south korea" => Some("Manhwa"),
		"china" => Some("Manhua"),
		"japan" => Some("Manga"),
		_ => None,
	}
}

pub fn viewer_for_origin(origin: &str) -> Viewer {
	match origin_format(origin) {
		Some("Manhwa" | "Manhua") => Viewer::Webtoon,
		Some("Manga") => Viewer::RightToLeft,
		_ => Viewer::Unknown,
	}
}

pub fn language_code(language: &str) -> String {
	match language.trim().to_ascii_lowercase().as_str() {
		"arabic" => "ar",
		"chinese" | "simplified chinese" | "traditional chinese" => "zh",
		"english" => "en",
		"french" => "fr",
		"german" => "de",
		"indonesian" => "id",
		"italian" => "it",
		"japanese" => "ja",
		"korean" => "ko",
		"polish" => "pl",
		"portuguese" => "pt",
		"brazilian portuguese" | "portuguese (brazil)" => "pt-BR",
		"russian" => "ru",
		"spanish" => "es",
		"thai" => "th",
		"turkish" => "tr",
		"vietnamese" => "vi",
		"unknown" => "unknown",
		_ => return language.trim().to_lowercase(),
	}
	.into()
}

fn relative_date(value: &str) -> Option<i64> {
	let normalized = value.trim().to_ascii_lowercase();
	let mut parts = normalized.split_whitespace();
	let amount: i64 = parts.next()?.parse().ok()?;
	let unit = parts.next()?;
	let seconds = if unit.starts_with("year") {
		31_536_000
	} else if unit.starts_with("month") {
		2_592_000
	} else if unit.starts_with("week") {
		604_800
	} else if unit.starts_with("day") {
		86_400
	} else if unit.starts_with("hour") {
		3_600
	} else if unit.starts_with("min") {
		60
	} else if unit.starts_with("sec") {
		1
	} else {
		return None;
	};
	Some(current_date() - amount * seconds)
}

pub fn parse_chapter_date(title: Option<&str>, text: Option<&str>) -> Option<i64> {
	title
		.filter(|value| !value.trim().is_empty())
		.and_then(|value| parse_date(value.trim(), "dd-MM-yyyy"))
		.or_else(|| {
			let text = text?.trim();
			parse_date(text, "MMM dd, yyyy")
				.or_else(|| parse_date(text, "MMMM dd, yyyy"))
				.or_else(|| relative_date(text))
		})
}
