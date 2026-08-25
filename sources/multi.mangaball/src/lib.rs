#![no_std]

use aidoku::{
	Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, ImageRequestProvider, Manga,
	MangaPageResult, Page, PageContent, PageContext, Result, Source,
	alloc::{String, Vec, format, string::ToString},
	helpers::uri::QueryParameters,
	imports::{
		net::{Request, TimeUnit, set_rate_limit},
		std::send_partial_result,
	},
	prelude::*,
};
use core::cell::RefCell;

mod helpers;
mod models;

use helpers::*;
use models::*;

const BASE_URL: &str = "https://mangaball.net";

struct MangaBall {
	csrf: RefCell<Option<String>>,
}

impl Source for MangaBall {
	fn new() -> Self {
		set_rate_limit(2, 1, TimeUnit::Seconds);
		Self {
			csrf: RefCell::new(None),
		}
	}

	fn get_search_manga_list(
		&self,
		query: Option<String>,
		page: i32,
		filters: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		let mut sort = "updated_chapters_desc";
		let mut demographic = "any";
		let mut status = "any";
		let mut include_mode = "and";
		let mut exclude_mode = "and";
		let mut included = Vec::new();
		let mut excluded = Vec::new();

		for filter in &filters {
			match filter {
				FilterValue::Sort { index, .. } => {
					sort = match index {
						0 => "updated_chapters_desc",
						1 => "updated_chapters_asc",
						2 => "created_at_desc",
						3 => "created_at_asc",
						4 => "name_asc",
						5 => "name_desc",
						6 => "views_desc",
						7 => "views_asc",
						_ => "updated_chapters_desc",
					};
				}
				FilterValue::Select { id, value } => match id.as_str() {
					"demographic" => demographic = value,
					"status" => status = value,
					"include_mode" => include_mode = value,
					"exclude_mode" => exclude_mode = value,
					_ => {}
				},
				FilterValue::MultiSelect {
					included: values,
					excluded: omitted,
					..
				} => {
					included.extend(values.iter());
					excluded.extend(omitted.iter());
				}
				_ => {}
			}
		}

		let mut body = QueryParameters::new();
		body.push("search_input", Some(query.as_deref().unwrap_or("").trim()));
		body.push("filters[sort]", Some(sort));
		body.push("filters[page]", Some(&page.to_string()));
		for id in included {
			body.push("filters[tag_included_ids][]", Some(id));
		}
		body.push("filters[tag_included_mode]", Some(include_mode));
		for id in excluded {
			body.push("filters[tag_excluded_ids][]", Some(id));
		}
		body.push("filters[tag_excluded_mode]", Some(exclude_mode));
		body.push("filters[contentRating]", Some("any"));
		body.push("filters[demographic]", Some(demographic));
		body.push("filters[person]", Some("any"));
		body.push("filters[publicationYear]", Some(""));
		body.push("filters[publicationStatus]", Some(status));
		for language in selected_languages() {
			body.push("filters[translatedLanguage][]", Some(&language));
		}

		let response: SearchResponse =
			self.post_json("/api/v1/title/search-advanced/", &body.to_string())?;
		Ok(response.into())
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		if needs_details {
			let html = get_request(&manga_url(&manga.key))?.html()?;
			self.remember_token(&html);
			fill_details(&html, &mut manga)?;
			if needs_chapters {
				send_partial_result(&manga);
			}
		}

		if needs_chapters {
			let title_id = manga
				.key
				.rsplit('-')
				.next()
				.ok_or_else(|| error!("Invalid manga key"))?;
			let mut body = QueryParameters::new();
			body.push("title_id", Some(title_id));
			let response: ChapterListResponse = self.post_json(
				"/api/v1/chapter/chapter-listing-by-title-id/",
				&body.to_string(),
			)?;
			let languages = selected_languages();
			manga.chapters = Some(
				response
					.chapters
					.into_iter()
					.flat_map(|container| {
						container.translations.into_iter().filter_map({
							let languages = languages.clone();
							move |translation| {
								languages
									.iter()
									.any(|language| language == &translation.language)
									.then(|| translation.into_chapter(container.number))
							}
						})
					})
					.collect(),
			);
		}

		Ok(manga)
	}

	fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let html = get_request(&chapter_url(&chapter.key))?.html()?;
		self.remember_token(&html);
		let script = html
			.select("script")
			.and_then(|scripts| {
				scripts
					.filter_map(|script| script.data())
					.find(|data| data.contains("chapterImages"))
			})
			.ok_or_else(|| error!("Chapter images not found"))?;
		Ok(parse_chapter_images(&script)?
			.into_iter()
			.map(|url| Page {
				content: PageContent::url(url),
				..Default::default()
			})
			.collect())
	}
}

impl DeepLinkHandler for MangaBall {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		let Some(path) = url.strip_prefix(BASE_URL) else {
			return Ok(None);
		};
		let path = path.split(['?', '#']).next().unwrap_or(path);
		let mut segments = path.split('/').filter(|segment| !segment.is_empty());
		Ok(match (segments.next(), segments.next()) {
			(Some("title-detail"), Some(key)) => Some(DeepLinkResult::Manga { key: key.into() }),
			(Some("chapter-detail"), Some(key)) => {
				let html = get_request(&url)?.html()?;
				let manga_key =
					chapter_manga_key(&html).ok_or_else(|| error!("Chapter manga not found"))?;
				Some(DeepLinkResult::Chapter {
					manga_key,
					key: key.into(),
				})
			}
			_ => None,
		})
	}
}

impl ImageRequestProvider for MangaBall {
	fn get_image_request(&self, url: String, _context: Option<PageContext>) -> Result<Request> {
		get_request(&url)
	}
}

fn manga_url(key: &str) -> String {
	format!("{BASE_URL}/title-detail/{key}/")
}

fn chapter_url(key: &str) -> String {
	format!("{BASE_URL}/chapter-detail/{key}/")
}

register_source!(MangaBall, DeepLinkHandler, ImageRequestProvider);

#[cfg(test)]
mod tests {
	use super::*;
	use aidoku_test::aidoku_test;

	#[aidoku_test]
	fn parses_chapter_images() {
		let pages = parse_chapter_images(
			r#"const chapterImages = JSON.parse(`["https://img/1.jpg","https://img/2.jpg"]`);"#,
		)
		.unwrap();
		assert_eq!(pages.len(), 2);
		assert_eq!(pages[0], "https://img/1.jpg");
	}

	#[aidoku_test]
	fn maps_language_aliases() {
		assert_eq!(api_languages("ja"), &["jp"]);
		assert!(api_languages("es").contains(&"es-419"));
		assert_eq!(api_languages("unknown"), &["en"]);
	}
}
