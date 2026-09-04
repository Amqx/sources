use aidoku::{
	alloc::{String, Vec, format, string::ToString},
	imports::{html::Element, std::current_date},
};

use crate::BASE_URL;

pub fn manga_slug(url: &str) -> Option<String> {
	let path = url.split_once("//").map_or(url, |(_, rest)| {
		rest.find('/').map_or("", |idx| &rest[idx..])
	});
	let rest = path.strip_prefix("/manga/")?;
	let slug = rest.split(['/', '?', '#']).next().unwrap_or(rest);
	(!slug.is_empty()).then(|| slug.into())
}

pub fn to_path(url: &str) -> String {
	url.split_once("//")
		.and_then(|(_, rest)| rest.find('/').map(|idx| rest[idx..].to_string()))
		.unwrap_or_else(|| url.to_string())
}

pub fn manga_url(key: &str) -> String {
	format!("{BASE_URL}/manga/{key}")
}

pub fn chapter_url(key: &str) -> String {
	if key.starts_with("http") {
		key.into()
	} else {
		format!("{BASE_URL}{key}")
	}
}

/// uses, and skipping inline `data:` placeholders.
pub fn image_url(img: &Element) -> Option<String> {
	["data-src", "data-lazy-src", "src"]
		.into_iter()
		.filter_map(|attr| img.attr(attr))
		.map(|src| src.trim().to_string())
		.find(|src| !src.is_empty() && !src.starts_with("data:"))
		.map(|src| absolute(&src))
}

pub fn absolute(url: &str) -> String {
	if url.starts_with("http") {
		url.into()
	} else if let Some(rest) = url.strip_prefix("//") {
		format!("https://{rest}")
	} else if url.starts_with('/') {
		format!("{BASE_URL}{url}")
	} else {
		format!("{BASE_URL}/{url}")
	}
}

pub fn split_details(text: &str) -> Vec<String> {
	text.replace(" - ", " · ")
		.split('·')
		.map(|part| part.trim().to_string())
		.filter(|part| !part.is_empty())
		.collect()
}

pub fn parse_relative_date(text: &str) -> Option<i64> {
	let text = text.trim().to_lowercase();
	if text.is_empty() {
		return None;
	}

	let now = current_date();
	if text.contains("today") {
		return Some(now);
	}
	if text.contains("yesterday") {
		return Some(now - 86_400);
	}
	if !text.ends_with("ago") {
		return None;
	}

	let mut words = text.split_whitespace();
	let value = words.next()?.parse::<i64>().ok()?;
	let seconds = match words.next()?.trim_end_matches('s') {
		"minute" => 60,
		"hour" => 3_600,
		"day" => 86_400,
		"week" => 604_800,
		"month" => 2_592_000,
		"year" => 31_536_000,
		_ => return None,
	};
	Some(now - value * seconds)
}

pub fn parse_chapter_number(text: &str) -> Option<f32> {
	let text = text.trim();
	let digits = text
		.strip_prefix("Chapter")
		.or_else(|| text.strip_prefix("chapter"))
		.unwrap_or(text)
		.trim();
	let end = digits
		.find(|c: char| !c.is_ascii_digit() && c != '.')
		.unwrap_or(digits.len());
	digits[..end].trim_end_matches('.').parse().ok()
}
