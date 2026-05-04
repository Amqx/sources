#![no_std]

use aidoku::{
	alloc::{String, Vec, format, rc::Rc, string::ToString},
	imports::{
		defaults::{DefaultValue, defaults_get, defaults_set},
		error::AidokuError,
		net::{Request, TimeUnit, set_rate_limit},
		std::current_date,
	},
	prelude::*,
	Chapter, FilterValue, ImageRequestProvider, Manga, MangaPageResult, Page, PageContent,
	PageContext, Result, Source,
};
use core::fmt::Write;

mod filters;
mod helpers;
mod models;
mod settings;
mod wvd;

use helpers::{build_chapter_name, parse_date, status_from_str};
use models::*;

const BASE_URL: &str = "https://kagane.org";
const API_URL: &str = "https://yuzuki.kagane.org";

const SOURCE_NUMBER_FORMATS: &[&str] = &[
	"Dark Horse Comics",
	"Flame Comics",
	"MangaDex",
	"Square Enix Manga",
];

const INTEGRITY_TOKEN_KEY: &str = "kagane_integrity_token";
const INTEGRITY_EXP_KEY: &str = "kagane_integrity_exp";

fn get_integrity_token() -> Result<String> {
	let now = current_date();
	let cached_exp = defaults_get::<String>(INTEGRITY_EXP_KEY)
		.and_then(|s| s.parse::<i64>().ok())
		.unwrap_or(0);

	if now < cached_exp
		&& let Some(token) = defaults_get::<String>(INTEGRITY_TOKEN_KEY)
			&& !token.is_empty() {
				return Ok(token);
			}

	let text = Request::post(format!("{BASE_URL}/api/integrity"))?
		.header("Content-Type", "application/json")
		.header("Origin", BASE_URL)
		.header("Referer", &format!("{BASE_URL}/"))
		.body("{}")
		.string()?;
	let dto: IntegrityDto =
		serde_json::from_str(&text).map_err(|e| AidokuError::JsonParseError(Rc::new(e)))?;

	defaults_set(INTEGRITY_TOKEN_KEY, DefaultValue::String(dto.token.clone()));
	defaults_set(INTEGRITY_EXP_KEY, DefaultValue::String(format!("{}", dto.exp)));

	Ok(dto.token)
}

struct Kagane;

fn pages_from_challenge(
	chapter_id: &str,
	challenge_dto: ChallengeDto,
	data_saver: bool,
) -> Result<Vec<Page>> {
	if challenge_dto.access_token.is_empty() || challenge_dto.cache_url.is_empty() {
		return Err(AidokuError::message("Invalid chapter access data"));
	}

	let cache_url = challenge_dto.cache_url;
	let access_token = challenge_dto.access_token;
	let mut pages = challenge_dto.pages;
	if pages.is_empty() {
		return Err(AidokuError::message("No pages found for this chapter"));
	}

	pages.sort_by_key(|p| p.page_number);

	pages
		.into_iter()
		.map(|page| {
			if page.page_uuid.is_empty() {
				return Err(AidokuError::message("Invalid chapter page data"));
			}

			let url = format!(
				"{cache_url}/api/v2/books/file/{chapter_id}/{}?token={access_token}&is_datasaver={data_saver}",
				page.page_uuid
			);
			Ok(Page {
				content: PageContent::url(url),
				..Default::default()
			})
		})
		.collect()
}

impl Source for Kagane {
	fn new() -> Self {
		set_rate_limit(2, 1, TimeUnit::Seconds);
		Self
	}

	fn get_search_manga_list(
		&self,
		query: Option<String>,
		page: i32,
		filter_values: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		let default_ratings = settings::get_default_content_ratings();
		let (body, sort_param) =
			filters::build_search_body(query.as_deref(), &filter_values, &default_ratings);

		let mut url = format!("{API_URL}/api/v2/search/series?page={}&size=35", page - 1);
		if !sort_param.is_empty() {
			let _ = write!(url, "&sort={sort_param}");
		}

		let text = Request::post(&url)?
			.header("Content-Type", "application/json")
			.header("Origin", BASE_URL)
			.header("Referer", &format!("{BASE_URL}/"))
			.body(body)
			.string()?;
		let dto: SearchDto =
			serde_json::from_str(&text).map_err(|e| AidokuError::JsonParseError(Rc::new(e)))?;

		let entries = dto
			.content
			.into_iter()
			.map(|book| Manga {
				key: book.series_id.clone(),
				title: book.title,
				cover: book.cover_image_id.map(|id| format!("{API_URL}/api/v2/image/{id}")),
				url: Some(format!("{BASE_URL}/series/{}", book.series_id)),
				..Default::default()
			})
			.collect();

		Ok(MangaPageResult {
			entries,
			has_next_page: !dto.last,
		})
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let url = format!("{API_URL}/api/v2/series/{}", manga.key);
		let text = Request::get(&url)?
			.header("Origin", BASE_URL)
			.header("Referer", &format!("{BASE_URL}/"))
			.string()?;
		let dto: DetailsDto =
			serde_json::from_str(&text).map_err(|e| AidokuError::JsonParseError(Rc::new(e)))?;

		if needs_details {
			let mut title = dto.title.trim().to_string();
			if settings::get_show_edition()
				&& let Some(ed) = dto.edition_info.as_deref().filter(|s| !s.is_empty()) {
					let _ = write!(title, " ({ed})");
				}
			if settings::get_show_source()
				&& let Some(src) = dto.source_id.as_deref().filter(|s| !s.is_empty()) {
					let _ = write!(title, " [{src}]");
				}
			manga.title = title;
			manga.status = status_from_str(&dto.upload_status);

			let authors: Vec<String> = dto
				.series_staff
				.iter()
				.filter(|s| {
					let role = s.role.to_ascii_lowercase();
					role.contains("author") || role.contains("story")
				})
				.map(|s| s.name.clone())
				.collect();
			if !authors.is_empty() {
				manga.authors = Some(authors);
			}

			let artists: Vec<String> = dto
				.series_staff
				.iter()
				.filter(|s| {
					let role = s.role.to_ascii_lowercase();
					role.contains("artist") || role.contains("art")
				})
				.map(|s| s.name.clone())
				.collect();
			if !artists.is_empty() {
				manga.artists = Some(artists);
			}

			let mut tags: Vec<String> = Vec::new();
			if let Some(fmt) = dto.format.as_deref().filter(|s| !s.is_empty()) {
				tags.push(fmt.to_string());
			}
			tags.extend(dto.genres.iter().map(|g| g.genre_name.clone()));
			if !tags.is_empty() {
				manga.tags = Some(tags);
			}

			let mut desc = String::new();
			if let Some(d) = dto.description.as_deref().filter(|s| !s.is_empty()) {
				desc.push_str(d.trim());
			}
			if !dto.series_alternate_titles.is_empty() {
				if !desc.is_empty() {
					desc.push_str("\n\n");
				}
				desc.push_str("Associated Names:\n");
				for alt in &dto.series_alternate_titles {
					let _ = writeln!(desc, "- {}", alt.title);
				}
			}
			if !desc.is_empty() {
				manga.description = Some(desc);
			}
		}

		if needs_chapters {
			let use_source_number = dto
				.format
				.as_deref()
				.map(|f| SOURCE_NUMBER_FORMATS.contains(&f))
				.unwrap_or(false);
			let mode = settings::get_chapter_title_mode();

			let chapters: Vec<Chapter> = dto
				.series_books
				.iter()
				.rev()
				.map(|book| {
					let key = format!("/series/{}/reader/{}", manga.key, book.book_id);
					let title = build_chapter_name(
						&book.title,
						book.chapter_no.as_deref(),
						book.volume_no.as_deref(),
						&mode,
					);
					let scanlators: Vec<String> =
						book.groups.iter().map(|g| g.title.clone()).collect();

					Chapter {
						key: key.clone(),
						title: if title.is_empty() { None } else { Some(title) },
						chapter_number: if use_source_number {
							Some(book.sort_no)
						} else {
							book.chapter_no.as_deref().and_then(|ch| ch.parse::<f32>().ok())
						},
						volume_number: book
							.volume_no
							.as_deref()
							.and_then(|vol| vol.parse::<f32>().ok()),
						date_uploaded: book.created_at.as_deref().map(|d| parse_date(d) as i64),
						scanlators: if scanlators.is_empty() {
							None
						} else {
							Some(scanlators)
						},
						url: Some(format!("{BASE_URL}{key}")),
						..Default::default()
					}
				})
				.collect();

			manga.chapters = Some(chapters);
		}

		Ok(manga)
	}

	fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let wvd_key = settings::get_wvd_key();
		if wvd_key.is_empty() {
			return Err(AidokuError::message(
				"WVD key required to read chapters. Add your WVD file (base64) in source settings.",
			));
		}

		let chapter_id = chapter
			.key
			.rsplit('/')
			.next()
			.filter(|s| !s.is_empty())
			.ok_or_else(|| AidokuError::message("Invalid chapter key format"))?;

		let integrity_token = get_integrity_token()?;
		let challenge = wvd::generate_challenge(&wvd_key, chapter_id)?;
		let data_saver = settings::get_data_saver();
		let challenge_url = format!("{API_URL}/api/v2/books/{chapter_id}?is_datasaver={data_saver}");
		let challenge_body = format!(r#"{{"challenge":"{challenge}"}}"#);

		let text = Request::post(&challenge_url)?
			.header("Content-Type", "application/json")
			.header("Origin", BASE_URL)
			.header("Referer", &format!("{BASE_URL}/"))
			.header("x-integrity-token", &integrity_token)
			.body(challenge_body)
			.string()?;
		let challenge_dto: ChallengeDto =
			serde_json::from_str(&text).map_err(|e| AidokuError::JsonParseError(Rc::new(e)))?;

		pages_from_challenge(chapter_id, challenge_dto, data_saver)
	}
}

impl ImageRequestProvider for Kagane {
	fn get_image_request(&self, url: String, _context: Option<PageContext>) -> Result<Request> {
		Ok(Request::get(&url)?
			.header("Origin", BASE_URL)
			.header("Referer", &format!("{BASE_URL}/")))
	}
}

register_source!(Kagane);

#[cfg(test)]
mod tests {
	use super::*;
	use aidoku::alloc::vec;
	use aidoku_test::aidoku_test;

	#[aidoku_test]
	fn test_empty_challenge_pages_is_error() {
		let result = pages_from_challenge(
			"chapter-id",
			ChallengeDto {
				access_token: "tok123".into(),
				cache_url: "https://akari.kagane.org".into(),
				pages: Vec::new(),
			},
			false,
		);

		assert!(result.is_err());
	}

	#[aidoku_test]
	fn test_challenge_pages_are_sorted_and_mapped() {
		let pages = pages_from_challenge(
			"chapter-id",
			ChallengeDto {
				access_token: "tok123".into(),
				cache_url: "https://akari.kagane.org".into(),
				pages: vec![
					PageDto {
						page_number: 2,
						page_uuid: "uuid-2".into(),
					},
					PageDto {
						page_number: 1,
						page_uuid: "uuid-1".into(),
					},
				],
			},
			true,
		)
		.unwrap();

		assert_eq!(pages.len(), 2);
		assert_eq!(
			pages[0].content,
			PageContent::Url(
				"https://akari.kagane.org/api/v2/books/file/chapter-id/uuid-1?token=tok123&is_datasaver=true".into(),
				None
			)
		);
		assert_eq!(
			pages[1].content,
			PageContent::Url(
				"https://akari.kagane.org/api/v2/books/file/chapter-id/uuid-2?token=tok123&is_datasaver=true".into(),
				None
			)
		);
	}

	#[aidoku_test]
	fn test_challenge_page_with_empty_uuid_is_error() {
		let result = pages_from_challenge(
			"chapter-id",
			ChallengeDto {
				access_token: "tok123".into(),
				cache_url: "https://akari.kagane.org".into(),
				pages: vec![PageDto {
					page_number: 1,
					page_uuid: String::new(),
				}],
			},
			false,
		);

		assert!(result.is_err());
	}

	#[aidoku_test]
	fn test_page_list_requires_wvd_key() {
		let source = Kagane::new();
		let result = source.get_page_list(
			Manga::default(),
			Chapter {
				key: "/series/series-id/reader/chapter-id".into(),
				..Default::default()
			},
		);
		assert!(result.is_err());
	}
}
