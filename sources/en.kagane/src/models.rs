use aidoku::alloc::{String, Vec};
use serde::{Deserialize, Deserializer};

#[derive(Deserialize)]
pub struct SearchDto {
	pub content: Vec<SearchBook>,
	pub last: bool,
}

#[derive(Deserialize)]
pub struct SearchBook {
	pub series_id: String,
	pub title: String,
	pub cover_image_id: Option<String>,
	#[allow(dead_code)]
	pub current_books: i32,
}

#[derive(Deserialize)]
pub struct DetailsDto {
	pub title: String,
	pub description: Option<String>,
	pub upload_status: String,
	pub format: Option<String>,
	pub source_id: Option<String>,
	#[serde(default)]
	pub series_staff: Vec<SeriesStaff>,
	#[serde(default)]
	pub genres: Vec<GenreItem>,
	#[serde(default)]
	pub series_alternate_titles: Vec<AlternateTitle>,
	#[serde(default)]
	pub series_books: Vec<BookDto>,
	pub edition_info: Option<String>,
}

#[derive(Deserialize)]
pub struct SeriesStaff {
	pub name: String,
	pub role: String,
}

#[derive(Deserialize)]
pub struct GenreItem {
	pub genre_name: String,
}

#[derive(Deserialize)]
pub struct AlternateTitle {
	pub title: String,
	#[allow(dead_code)]
	pub label: Option<String>,
}

#[derive(Deserialize)]
pub struct BookDto {
	pub book_id: String,
	pub title: String,
	pub created_at: Option<String>,
	#[allow(dead_code)]
	pub page_count: i32,
	pub sort_no: f32,
	pub chapter_no: Option<String>,
	pub volume_no: Option<String>,
	#[serde(default)]
	pub groups: Vec<GroupDto>,
}

#[derive(Deserialize)]
pub struct GroupDto {
	pub title: String,
}

pub struct ChallengeDto {
	pub access_token: String,
	pub cache_url: String,
	pub pages: Vec<PageDto>,
}

#[derive(Deserialize)]
struct ChallengeRawDto {
	pub access_token: String,
	pub cache_url: String,
	#[serde(default)]
	pub pages: Vec<PageDto>,
	pub manifest: Option<ChallengeManifestDto>,
}

#[derive(Deserialize)]
struct ChallengeManifestDto {
	#[serde(default)]
	pub pages: Vec<PageDto>,
}

impl<'de> Deserialize<'de> for ChallengeDto {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let raw = ChallengeRawDto::deserialize(deserializer)?;
		let pages = if raw.pages.is_empty() {
			raw.manifest.map(|m| m.pages).unwrap_or_default()
		} else {
			raw.pages
		};

		Ok(Self {
			access_token: raw.access_token,
			cache_url: raw.cache_url,
			pages,
		})
	}
}

#[derive(Deserialize)]
pub struct PageDto {
	#[serde(alias = "page_no")]
	pub page_number: i32,
	#[serde(alias = "page_id")]
	pub page_uuid: String,
}

#[derive(Deserialize)]
pub struct IntegrityDto {
	pub token: String,
	pub exp: i64,
}

#[cfg(test)]
mod tests {
	use super::*;
	use aidoku_test::aidoku_test;

	#[aidoku_test]
	fn test_search_dto_parse() {
		let json = r#"{
            "content": [{
                "series_id": "abc-123",
                "title": "Test Manga",
                "cover_image_id": "cover-uuid",
                "current_books": 5
            }],
            "last": false,
            "total_elements": 100,
            "total_pages": 3
        }"#;
		let dto: SearchDto = serde_json::from_str(json).unwrap();
		assert_eq!(dto.content[0].series_id, "abc-123");
		assert_eq!(dto.content[0].title, "Test Manga");
		assert_eq!(dto.last, false);
	}

	#[aidoku_test]
	fn test_details_dto_parse() {
		let json = r#"{
            "title": "My Manga",
            "description": "A story.",
            "upload_status": "Ongoing",
            "format": "Manga",
            "source_id": "src-1",
            "series_staff": [{"name": "Author Name", "role": "Author"}],
            "genres": [{"genre_name": "Action"}],
            "series_alternate_titles": [],
            "series_books": [{
                "book_id": "book-1",
                "title": "Chapter 1",
                "created_at": "2024-01-15T10:30:00",
                "page_count": 20,
                "sort_no": 1.0,
                "chapter_no": "1",
                "volume_no": null,
                "groups": []
            }],
            "edition_info": null
        }"#;
		let dto: DetailsDto = serde_json::from_str(json).unwrap();
		assert_eq!(dto.title, "My Manga");
		assert_eq!(dto.series_staff[0].name, "Author Name");
		assert_eq!(dto.series_books[0].book_id, "book-1");
	}

	#[aidoku_test]
	fn test_challenge_dto_parse() {
		let json = r#"{
            "access_token": "tok123",
            "cache_url": "https://akari.kagane.org",
            "pages": [
                {"page_number": 1, "page_uuid": "uuid-1"},
                {"page_number": 2, "page_uuid": "uuid-2"}
            ]
        }"#;
		let dto: ChallengeDto = serde_json::from_str(json).unwrap();
		assert_eq!(dto.access_token, "tok123");
		assert_eq!(dto.pages.len(), 2);
		assert_eq!(dto.pages[0].page_uuid, "uuid-1");
	}

	#[aidoku_test]
	fn test_challenge_dto_parse_manifest_pages() {
		let json = r#"{
            "access_token": "tok123",
            "cache_url": "https://kstatic.to",
            "manifest": {
                "pages": [
                    {"page_id": "uuid-1", "page_no": 1, "ext": "jxl", "height": 4267, "width": 1280},
                    {"page_id": "uuid-2", "page_no": 2, "ext": "jxl", "height": 4267, "width": 1280}
                ],
                "version": 1
            }
        }"#;
		let dto: ChallengeDto = serde_json::from_str(json).unwrap();
		assert_eq!(dto.access_token, "tok123");
		assert_eq!(dto.pages.len(), 2);
		assert_eq!(dto.pages[0].page_uuid, "uuid-1");
		assert_eq!(dto.pages[0].page_number, 1);
	}

	#[aidoku_test]
	fn test_integrity_dto_parse() {
		let json = r#"{"token": "integ-tok", "exp": 1700000000}"#;
		let dto: IntegrityDto = serde_json::from_str(json).unwrap();
		assert_eq!(dto.token, "integ-tok");
		assert_eq!(dto.exp, 1700000000);
	}
}
