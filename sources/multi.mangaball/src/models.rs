use crate::{chapter_url, manga_url};
use aidoku::{
	Chapter, Manga, MangaPageResult,
	alloc::{String, Vec, format, string::ToString, vec},
	imports::std::parse_date,
};
use serde::{Deserialize, Deserializer};

#[derive(Deserialize)]
pub struct SearchResponse {
	#[serde(default)]
	pub data: Vec<SearchManga>,
	pub pagination: Option<Pagination>,
}

#[derive(Deserialize)]
pub struct Pagination {
	pub current_page: i32,
	pub last_page: i32,
}

#[derive(Deserialize)]
pub struct SearchManga {
	pub url: String,
	pub name: String,
	pub cover: Option<String>,
}

impl From<SearchManga> for Manga {
	fn from(value: SearchManga) -> Self {
		let key = manga_key(&value.url).unwrap_or(value.url);
		Self {
			url: Some(manga_url(&key)),
			key,
			title: value.name,
			cover: value.cover,
			..Default::default()
		}
	}
}

impl From<SearchResponse> for MangaPageResult {
	fn from(value: SearchResponse) -> Self {
		Self {
			entries: value.data.into_iter().map(Into::into).collect(),
			has_next_page: value
				.pagination
				.is_some_and(|page| page.current_page < page.last_page),
		}
	}
}

fn manga_key(url: &str) -> Option<String> {
	url.split_once("/title-detail/")
		.and_then(|(_, path)| path.split('/').find(|segment| !segment.is_empty()))
		.map(Into::into)
}

#[derive(Deserialize)]
pub struct ChapterListResponse {
	#[serde(rename = "ALL_CHAPTERS", default)]
	pub chapters: Vec<ChapterContainer>,
}

#[derive(Deserialize)]
pub struct ChapterContainer {
	#[serde(rename = "number_float", deserialize_with = "f32_from_any")]
	pub number: f32,
	#[serde(default)]
	pub translations: Vec<ChapterTranslation>,
}

#[derive(Deserialize)]
pub struct ChapterTranslation {
	pub id: String,
	#[serde(default)]
	pub name: String,
	pub language: String,
	pub group: ChapterGroup,
	pub date: Option<String>,
	#[serde(default, deserialize_with = "f32_from_any")]
	pub volume: f32,
}

#[derive(Deserialize)]
pub struct ChapterGroup {
	#[serde(rename = "_id", default)]
	pub id: String,
	#[serde(default)]
	pub name: String,
}

impl ChapterTranslation {
	pub fn into_chapter(self, number: f32) -> Chapter {
		let mut title_parts = Vec::new();
		if self.volume > 0.0 {
			title_parts.push(format!("Vol. {}", display_number(self.volume)));
		}
		let chapter_number = display_number(number);
		if self.name.contains(&chapter_number) {
			title_parts.push(self.name.trim().into());
		} else if self.name.trim().is_empty() {
			title_parts.push(format!("Ch. {chapter_number}"));
		} else {
			title_parts.push(format!("Ch. {chapter_number} {}", self.name.trim()));
		}

		let mut group = self.group.name;
		if !self.group.id.is_empty() && !is_object_id(&self.group.id) {
			if !group.is_empty() {
				group.push_str(" (");
				group.push_str(&self.group.id);
				group.push(')');
			} else {
				group = self.group.id;
			}
		}

		Chapter {
			key: self.id.clone(),
			title: Some(title_parts.join(" ")),
			chapter_number: Some(number),
			volume_number: (self.volume > 0.0).then_some(self.volume),
			date_uploaded: self
				.date
				.as_deref()
				.and_then(|date| parse_date(date, "yyyy-MM-dd HH:mm:ss")),
			scanlators: (!group.is_empty()).then(|| vec![group]),
			url: Some(chapter_url(&self.id)),
			language: Some(self.language),
			..Default::default()
		}
	}
}

fn f32_from_any<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
	D: Deserializer<'de>,
{
	let value = serde_json::Value::deserialize(deserializer)?;
	Ok(match value {
		serde_json::Value::Number(number) => number.as_f64().unwrap_or_default() as f32,
		serde_json::Value::String(text) => text.parse().unwrap_or_default(),
		_ => 0.0,
	})
}

fn display_number(number: f32) -> String {
	if number == number as i32 as f32 {
		format!("{}", number as i32)
	} else {
		number.to_string()
	}
}

fn is_object_id(value: &str) -> bool {
	value.len() == 24 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
	use super::*;
	use aidoku_test::aidoku_test;

	#[aidoku_test]
	fn accepts_string_volume() {
		let chapter: ChapterTranslation = serde_json::from_str(
			r#"{"id":"abc","name":"","language":"en","group":{"_id":"site","name":"Group"},"date":"2026-01-02 03:04:05","volume":"2.0"}"#,
		)
		.unwrap();
		assert_eq!(chapter.volume, 2.0);
	}

	#[aidoku_test]
	fn extracts_search_key() {
		assert_eq!(
			manga_key("https://mangaball.net/title-detail/one-piece-abc123/"),
			Some("one-piece-abc123".into())
		);
		assert_eq!(
			manga_key("http://mangaball.net/title-detail/one-piece-abc123/"),
			Some("one-piece-abc123".into())
		);
		assert_eq!(
			manga_key("/title-detail/one-piece-abc123/"),
			Some("one-piece-abc123".into())
		);
	}
}
