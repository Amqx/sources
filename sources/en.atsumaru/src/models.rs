use aidoku::{
	Chapter, ContentRating, Manga,
	alloc::{String, Vec, borrow::ToOwned, format},
	imports::std::parse_date,
};
use serde::Deserialize;

use crate::helpers::*;

// ── Listings ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListingResponse {
	#[serde(default)]
	pub items: Vec<ListingItem>,
}

/// An entry as returned by the infinite scroll listing endpoints.
#[derive(Deserialize)]
pub struct ListingItem {
	pub id: String,
	pub title: String,
	pub image: Option<String>,
	#[serde(rename = "isAdult")]
	pub is_adult: Option<bool>,
	#[serde(rename = "type")]
	pub kind: Option<String>,
}

impl From<ListingItem> for Manga {
	fn from(item: ListingItem) -> Self {
		Manga {
			url: Some(format!("{BASE_URL}/manga/{}", item.id)),
			key: item.id,
			title: item.title,
			cover: item.image.as_deref().map(image_url),
			content_rating: if item.is_adult.unwrap_or(false) {
				ContentRating::NSFW
			} else {
				ContentRating::Unknown
			},
			viewer: parse_viewer(item.kind.as_deref()),
			..Default::default()
		}
	}
}

// ── Search ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SearchResponse {
	#[serde(default)]
	pub found: i32,
	#[serde(default)]
	pub hits: Vec<SearchHit>,
}

#[derive(Deserialize)]
pub struct SearchHit {
	pub document: SearchDocument,
}

/// A search index document. It holds most of what a manga page does, so search
/// results are filled in as far as they go.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDocument {
	pub id: String,
	pub title: String,
	pub poster: Option<String>,
	pub synopsis: Option<String>,
	#[serde(default)]
	pub authors: Vec<String>,
	#[serde(default)]
	pub tags: Vec<String>,
	pub status: Option<String>,
	#[serde(rename = "type")]
	pub kind: Option<String>,
	pub is_adult: Option<bool>,
	pub mb_content_rating: Option<String>,
}

impl From<SearchDocument> for Manga {
	fn from(document: SearchDocument) -> Self {
		let is_adult = document.is_adult.unwrap_or(false);
		let viewer = parse_viewer(document.kind.as_deref());

		let mut tags = document.tags;
		let content_rating = parse_mb_content_rating(document.mb_content_rating.as_deref())
			.filter(|_| !is_adult)
			.unwrap_or_else(|| parse_content_rating(is_adult, &tags));
		if let Some(kind) = document.kind {
			tags.insert(0, kind);
		}

		Manga {
			url: Some(format!("{BASE_URL}/manga/{}", document.id)),
			key: document.id,
			title: document.title,
			cover: document.poster.as_deref().map(image_url),
			description: document.synopsis.filter(|s| !s.trim().is_empty()),
			authors: (!document.authors.is_empty()).then_some(document.authors),
			tags: (!tags.is_empty()).then_some(tags),
			status: parse_status(document.status.as_deref()),
			content_rating,
			viewer,
			..Default::default()
		}
	}
}

// ── Manga page ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaPageResponse {
	pub manga_page: MangaPage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaPage {
	pub title: String,
	pub poster: Option<Poster>,
	#[serde(default)]
	pub authors: Vec<Author>,
	pub synopsis: Option<String>,
	#[serde(default)]
	pub genres: Vec<NamedItem>,
	pub released: Option<i64>,
	pub status: Option<String>,
	#[serde(rename = "type")]
	pub kind: Option<String>,
	pub views: Option<String>,
	#[serde(default)]
	pub other_names: Vec<String>,
	pub avg_rating: Option<f64>,
	pub is_adult: Option<bool>,
	pub scanlators: Option<Vec<Scanlator>>,
	pub chapters: Option<Vec<ChapterItem>>,
	pub has_more_chapters: Option<bool>,
}

#[derive(Deserialize)]
pub struct Poster {
	pub image: Option<String>,
}

#[derive(Deserialize)]
pub struct Author {
	pub name: String,
	#[serde(rename = "type")]
	pub kind: Option<String>,
}

#[derive(Deserialize)]
pub struct NamedItem {
	pub name: String,
}

#[derive(Deserialize)]
pub struct Scanlator {
	pub id: String,
	pub name: String,
}

impl MangaPage {
	/// Fills the details of a manga, leaving its key and chapters alone.
	pub fn fill_details(self, manga: &mut Manga) {
		manga.title = self.title;
		// entries without a poster keep whatever cover they were listed with
		if let Some(image) = self.poster.and_then(|poster| poster.image) {
			manga.cover = Some(image_url(&image));
		}
		manga.url = Some(format!("{BASE_URL}/manga/{}", manga.key));
		manga.status = parse_status(self.status.as_deref());
		manga.viewer = parse_viewer(self.kind.as_deref());

		let mut authors: Vec<String> = Vec::new();
		let mut artists: Vec<String> = Vec::new();
		for author in self.authors {
			match author.kind.as_deref() {
				Some("Artist") => artists.push(author.name),
				Some("Author") | None => authors.push(author.name),
				_ => {}
			}
		}
		manga.authors = (!authors.is_empty()).then_some(authors);
		manga.artists = (!artists.is_empty()).then_some(artists);

		let mut tags: Vec<String> = Vec::with_capacity(self.genres.len() + 1);
		if let Some(kind) = self.kind {
			tags.push(kind);
		}
		tags.extend(self.genres.into_iter().map(|genre| genre.name));
		manga.content_rating = parse_content_rating(self.is_adult.unwrap_or(false), &tags);
		manga.tags = (!tags.is_empty()).then_some(tags);

		// The website shows these stats next to the synopsis, and Aidoku has
		// nowhere else to put them.
		let mut stats: Vec<String> = Vec::new();
		if let Some(rating) = self.avg_rating.filter(|rating| *rating > 0.0) {
			stats.push(format!("Rating: {rating:.2}/10"));
		}
		if let Some(released) = self.released.filter(|released| *released > 0) {
			stats.push(format!("Year: {}", year_from_millis(released)));
		}
		if let Some(views) = self.views {
			stats.push(format!("Views: {views}"));
		}

		let mut sections: Vec<String> = Vec::new();
		if !stats.is_empty() {
			sections.push(stats.join("\n"));
		}
		if let Some(synopsis) = self.synopsis.filter(|s| !s.trim().is_empty()) {
			sections.push(synopsis.trim().to_owned());
		}
		let other_names = self
			.other_names
			.iter()
			.filter(|name| **name != manga.title)
			.map(|name| format!("- {name}"))
			.collect::<Vec<String>>();
		if !other_names.is_empty() {
			sections.push(format!("Alternative Names:\n{}", other_names.join("\n")));
		}
		manga.description = (!sections.is_empty()).then(|| sections.join("\n\n"));
	}
}

// ── Chapters ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AllChaptersResponse {
	#[serde(default)]
	pub chapters: Vec<ChapterItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterItem {
	pub id: String,
	pub title: Option<String>,
	pub number: Option<f32>,
	pub scanlation_manga_id: Option<String>,
	pub created_at: Option<UploadDate>,
}

/// Upload dates are usually a unix timestamp in milliseconds, but older entries
/// hold an ISO 8601 string instead.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum UploadDate {
	Millis(i64),
	Text(String),
}

impl UploadDate {
	fn timestamp(&self) -> Option<i64> {
		match self {
			Self::Millis(millis) => Some(millis / 1000),
			Self::Text(text) => parse_date(text, "yyyy-MM-dd'T'HH:mm:ss.SSS'Z'"),
		}
	}
}

impl ChapterItem {
	pub fn into_chapter(self, manga_key: &str, scanlators: &[Scanlator]) -> Chapter {
		let scanlator = self.scanlation_manga_id.and_then(|id| {
			scanlators
				.iter()
				.find(|scanlator| scanlator.id == id)
				.map(|scanlator| scanlator.name.clone())
		});

		Chapter {
			url: Some(format!("{BASE_URL}/read/{manga_key}/{}", self.id)),
			title: clean_chapter_title(self.title, self.number),
			chapter_number: self.number,
			date_uploaded: self.created_at.as_ref().and_then(UploadDate::timestamp),
			scanlators: scanlator.map(|scanlator| Vec::from([scanlator])),
			key: self.id,
			..Default::default()
		}
	}
}

// ── Pages ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadChapterResponse {
	pub read_chapter: ReadChapter,
}

#[derive(Deserialize)]
pub struct ReadChapter {
	#[serde(default)]
	pub pages: Vec<PageItem>,
}

#[derive(Deserialize)]
pub struct PageItem {
	pub image: String,
}
