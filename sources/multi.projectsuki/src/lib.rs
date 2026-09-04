#![no_std]

mod helpers;
mod models;
mod settings;

use aidoku::{
	Chapter, ContentRating, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeComponent,
	HomeComponentValue, HomeLayout, ImageRequestProvider, Listing, ListingProvider, Manga,
	MangaPageResult, MangaWithChapter, Page, PageContent, PageContext, Result, Source,
	UpdateStrategy,
	alloc::{String, Vec, format, string::ToString, vec},
	helpers::uri::QueryParameters,
	imports::{
		html::{Document, Element, Html},
		net::{Request, TimeUnit, set_rate_limit},
		std::send_partial_result,
	},
	prelude::*,
};

use helpers::*;
use models::{ChapterPagesResponse, SearchResponse};

const USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";

struct ProjectSuki;

fn request(url: &str) -> Result<Request> {
	Ok(Request::get(url)?
		.header("User-Agent", USER_AGENT)
		.header("Referer", &format!("{BASE_URL}/")))
}

fn fetch_html(url: &str) -> Result<Document> {
	Ok(request(url)?.html()?)
}

fn parse_manga_list(html: &Document) -> MangaPageResult {
	let entries = html
		.select("div.browse:has(.details a[href^='/book/'])")
		.map(|elements| {
			elements
				.filter_map(|element| {
					let link = element.select_first(
						".details h4 a[href^='/book/'], .details h5 a[href^='/book/']",
					)?;
					let href = link.attr("href")?;
					let key = manga_key_from_url(&href)?;
					let title = link.text()?;
					let cover = element
						.select_first("img")
						.and_then(|image| image.attr("abs:src").or_else(|| image.attr("src")))
						.map(|url| absolute_url(&url));

					Some(Manga {
						key: key.clone(),
						title,
						cover,
						url: Some(manga_url(&key)),
						content_rating: ContentRating::Safe,
						..Default::default()
					})
				})
				.collect()
		})
		.unwrap_or_default();

	MangaPageResult {
		entries,
		has_next_page: html.select_first(".pagination a.pull-right").is_some(),
	}
}

fn active_advanced_filters(filters: &[FilterValue]) -> bool {
	filters.iter().any(|filter| match filter {
		FilterValue::Text { id, value } => {
			matches!(id.as_str(), "author" | "artist") && !value.trim().is_empty()
		}
		FilterValue::Select { id, value } => {
			matches!(id.as_str(), "origin" | "status") && !value.is_empty()
		}
		_ => false,
	})
}

fn full_search_url(query: Option<&str>, page: i32, filters: &[FilterValue]) -> String {
	let mut parameters = QueryParameters::new();
	let page = (page - 1).max(0).to_string();
	parameters.push("page", Some(&page));
	parameters.push("q", Some(query.unwrap_or_default().trim()));

	let advanced = active_advanced_filters(filters);
	if advanced {
		parameters.push("adv", Some("1"));
	}

	for filter in filters {
		match filter {
			FilterValue::Text { id, value }
				if matches!(id.as_str(), "author" | "artist") && !value.trim().is_empty() =>
			{
				parameters.push(id, Some(value.trim()));
			}
			FilterValue::Select { id, value }
				if matches!(id.as_str(), "origin" | "status") && !value.is_empty() =>
			{
				parameters.push(id, Some(value));
			}
			_ => {}
		}
	}

	format!("{BASE_URL}/search?{parameters}")
}

fn smart_search(query: &str) -> Result<MangaPageResult> {
	let response = Request::post(format!("{BASE_URL}/api/book/search"))?
		.header("Content-Type", "application/json")
		.header("X-Requested-With", "XMLHttpRequest")
		.header("User-Agent", USER_AGENT)
		.header("Referer", &format!("{BASE_URL}/browse"))
		.body("{\"hash\":null}")
		.json_owned::<SearchResponse>()?;

	let words: Vec<String> = query
		.to_lowercase()
		.split_whitespace()
		.map(String::from)
		.collect();
	if words.is_empty() {
		return Ok(MangaPageResult::default());
	}

	let mut matches: Vec<(usize, String, String)> = response
		.data
		.into_iter()
		.filter_map(|(key, book)| {
			let normalized = book.value.to_lowercase();
			let score = words
				.iter()
				.map(|word| normalized.matches(word).count() * word.chars().count().max(1))
				.sum();
			(score > 0).then_some((score, key, book.value))
		})
		.collect();
	matches.sort_by(|left, right| {
		right
			.0
			.cmp(&left.0)
			.then_with(|| left.2.to_lowercase().cmp(&right.2.to_lowercase()))
	});

	let highest = matches.first().map(|item| item.0).unwrap_or_default();
	let mut entries = Vec::new();
	for (index, (score, key, title)) in matches.into_iter().enumerate() {
		if index >= 50 || (index >= 8 && score.saturating_mul(2) < highest) {
			break;
		}
		entries.push(Manga {
			cover: Some(thumbnail_url(&key)),
			url: Some(manga_url(&key)),
			key,
			title,
			content_rating: ContentRating::Safe,
			..Default::default()
		});
	}

	Ok(MangaPageResult {
		entries,
		has_next_page: false,
	})
}

fn collect_text(element: &Document, selector: &str) -> Option<Vec<String>> {
	let values: Vec<String> = element
		.select(selector)?
		.filter_map(|element| element.text())
		.filter(|text| !text.is_empty())
		.collect();
	(!values.is_empty()).then_some(values)
}

fn detail_value(html: &Document, label: &str) -> Option<String> {
	html.select(".row")?.find_map(|row| {
		let mut children = row.children();
		let heading = children.first()?.text()?;
		if heading.trim_end_matches(':').eq_ignore_ascii_case(label) {
			children.next_back()?.text()
		} else {
			None
		}
	})
}

fn parse_chapter(element: &Element) -> Option<Chapter> {
	let link = element.select_first("a[href^='/read/']")?;
	let href = link.attr("href")?;
	let (manga_key, chapter_id) = chapter_keys_from_url(&href)?;
	let title = link.text().unwrap_or_default();
	let raw_language = element
		.select_first("span[itemtype$=language]")
		.and_then(|element| element.text())
		.unwrap_or_else(|| "Unknown".into());
	if !settings::language_allowed(&raw_language) {
		return None;
	}

	let date_element = element.select_first("span[itemtype$=dateCreated]");
	let date_uploaded = date_element
		.as_ref()
		.and_then(|date| parse_chapter_date(date.attr("title").as_deref(), date.text().as_deref()));
	let scanlators = element
		.select_first("a[href^='/group/']")
		.and_then(|group| group.text())
		.filter(|group| !group.is_empty())
		.map(|group| vec![group]);

	Some(Chapter {
		key: chapter_key(&manga_key, &chapter_id),
		title: Some(title.clone()),
		chapter_number: chapter_number(&title),
		date_uploaded,
		scanlators,
		url: Some(absolute_url(&href)),
		language: Some(language_code(&raw_language)),
		..Default::default()
	})
}

fn latest_entries(html: &Document) -> Vec<MangaWithChapter> {
	html.select("div.item:has(a[href^='/read/'])")
		.map(|elements| {
			elements
				.filter_map(|element| {
					let manga_link = element.select_first("a[itemprop=title][href^='/book/']")?;
					let manga_href = manga_link.attr("href")?;
					let manga_key = manga_key_from_url(&manga_href)?;
					let manga = Manga {
						key: manga_key.clone(),
						title: manga_link.text()?,
						cover: element
							.select_first("img")
							.and_then(|image| image.attr("abs:src").or_else(|| image.attr("src")))
							.map(|url| absolute_url(&url)),
						url: Some(manga_url(&manga_key)),
						content_rating: ContentRating::Safe,
						..Default::default()
					};
					let chapter = parse_chapter(&element)?;
					Some(MangaWithChapter { manga, chapter })
				})
				.collect()
		})
		.unwrap_or_default()
}

impl Source for ProjectSuki {
	fn new() -> Self {
		set_rate_limit(2, 1, TimeUnit::Seconds);
		Self
	}

	fn get_search_manga_list(
		&self,
		query: Option<String>,
		page: i32,
		filters: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		if let Some(query) = query
			.as_deref()
			.map(str::trim)
			.filter(|query| !query.is_empty())
			&& !active_advanced_filters(&filters)
		{
			return smart_search(query);
		}

		if query
			.as_deref()
			.is_some_and(|query| !query.trim().is_empty())
			|| active_advanced_filters(&filters)
		{
			return Ok(parse_manga_list(&fetch_html(&full_search_url(
				query.as_deref(),
				page,
				&filters,
			))?));
		}

		Ok(parse_manga_list(&fetch_html(&format!(
			"{BASE_URL}/browse/{}",
			(page - 1).max(0)
		))?))
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let url = manga_url(&manga.key);
		let html = fetch_html(&url)?;

		if needs_details {
			manga.title = html
				.select_first("h2[itemprop=title]")
				.and_then(|element| element.text())
				.unwrap_or(manga.title);
			manga.cover = html
				.select_first("img.img-thumbnail")
				.and_then(|image| image.attr("abs:src").or_else(|| image.attr("src")))
				.map(|url| absolute_url(&url));
			manga.authors = collect_text(&html, "a[href*='author=']");
			manga.artists = collect_text(&html, "a[href*='artist=']");
			manga.status = html
				.select_first("a[href*='status=']")
				.and_then(|element| element.text())
				.map(|status| manga_status(&status))
				.unwrap_or_default();

			let mut tags = collect_text(&html, "a[href^='/genre/']").unwrap_or_default();
			if let Some(origin) = html
				.select_first("a[href*='origin=']")
				.and_then(|element| element.text())
			{
				if let Some(format) = origin_format(&origin)
					&& !tags.iter().any(|tag| tag == format)
				{
					tags.push(format.into());
				}
				manga.viewer = viewer_for_origin(&origin);
			}
			if !tags.is_empty() {
				manga.tags = Some(tags);
			}

			let description = html
				.select_first("#descriptionCollapse")
				.and_then(|element| element.text())
				.unwrap_or_default();
			manga.description = match detail_value(&html, "Alternative titles") {
				Some(alternative_titles)
					if !alternative_titles.is_empty() && !description.is_empty() =>
				{
					Some(format!(
						"Alternative titles: {alternative_titles}\n\n{description}"
					))
				}
				Some(alternative_titles) if !alternative_titles.is_empty() => {
					Some(format!("Alternative titles: {alternative_titles}"))
				}
				_ if !description.is_empty() => Some(description),
				_ => None,
			};
			manga.url = Some(url.clone());
			manga.content_rating = ContentRating::Safe;
			manga.update_strategy = match manga.status {
				aidoku::MangaStatus::Completed | aidoku::MangaStatus::Cancelled => {
					UpdateStrategy::Never
				}
				_ => UpdateStrategy::Always,
			};

			if needs_chapters {
				send_partial_result(&manga);
			}
		}

		if needs_chapters {
			manga.chapters = html
				.select("table:has(a[href^='/read/']) tbody tr")
				.map(|rows| rows.filter_map(|row| parse_chapter(&row)).collect());
		}

		Ok(manga)
	}

	fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let (book_id, chapter_id) = split_chapter_key(&chapter.key)
			.ok_or_else(|| error!("Invalid chapter key: {}", chapter.key))?;
		let body = format!(
			"{{\"bookid\":\"{book_id}\",\"chapterid\":\"{chapter_id}\",\"first\":\"true\"}}"
		);
		let response = Request::post(format!("{BASE_URL}/callpage"))?
			.header("Content-Type", "application/json")
			.header("X-Requested-With", "XMLHttpRequest")
			.header("User-Agent", USER_AGENT)
			.header("Referer", chapter.url.as_deref().unwrap_or(BASE_URL))
			.body(body)
			.json_owned::<ChapterPagesResponse>()?;
		let fragment = Html::parse_with_url(response.src, BASE_URL)?;
		let pages: Vec<Page> = fragment
			.select("img[src]")
			.map(|images| {
				images
					.filter_map(|image| image.attr("abs:src").or_else(|| image.attr("src")))
					.map(|url| Page {
						content: PageContent::url(absolute_url(&url)),
						..Default::default()
					})
					.collect()
			})
			.unwrap_or_default();

		if pages.is_empty() {
			bail!("No chapter pages found");
		}
		Ok(pages)
	}
}

impl ListingProvider for ProjectSuki {
	fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
		match listing.id.as_str() {
			"browse" => Ok(parse_manga_list(&fetch_html(&format!(
				"{BASE_URL}/browse/{}",
				(page - 1).max(0)
			))?)),
			"latest" if page <= 1 => {
				let entries = latest_entries(&fetch_html(BASE_URL)?)
					.into_iter()
					.map(|entry| entry.manga)
					.collect();
				Ok(MangaPageResult {
					entries,
					has_next_page: false,
				})
			}
			"latest" => Ok(MangaPageResult::default()),
			id if id.starts_with("search:") => {
				let query = id.strip_prefix("search:").unwrap_or_default();
				let separator = if query.is_empty() { "" } else { "&" };
				let url = format!(
					"{BASE_URL}/search?{query}{separator}page={}",
					(page - 1).max(0)
				);
				Ok(parse_manga_list(&fetch_html(&url)?))
			}
			_ => bail!("Unknown listing: {}", listing.id),
		}
	}
}

impl Home for ProjectSuki {
	fn get_home(&self) -> Result<HomeLayout> {
		let entries = latest_entries(&fetch_html(BASE_URL)?);
		Ok(HomeLayout {
			components: vec![HomeComponent {
				title: Some("Latest Chapters".into()),
				subtitle: None,
				value: HomeComponentValue::MangaChapterList {
					page_size: Some(6),
					entries,
					listing: Some(Listing {
						id: "latest".into(),
						name: "Latest".into(),
						..Default::default()
					}),
				},
			}],
		})
	}
}

impl DeepLinkHandler for ProjectSuki {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		let Some(relative) = url.strip_prefix(BASE_URL) else {
			return Ok(None);
		};
		let relative = relative.split('#').next().unwrap_or(relative);
		let (path, query) = relative.split_once('?').unwrap_or((relative, ""));
		let segments: Vec<&str> = path
			.split('/')
			.filter(|segment| !segment.is_empty())
			.collect();

		Ok(match segments.as_slice() {
			["book", manga_key] if valid_id(manga_key) => Some(DeepLinkResult::Manga {
				key: (*manga_key).into(),
			}),
			["read", manga_key, chapter_id, ..] if valid_id(manga_key) && valid_id(chapter_id) => {
				Some(DeepLinkResult::Chapter {
					manga_key: (*manga_key).into(),
					key: chapter_key(manga_key, chapter_id),
				})
			}
			["search"] => Some(DeepLinkResult::Listing(Listing {
				id: format!("search:{query}"),
				name: "Search Results".into(),
				..Default::default()
			})),
			_ => None,
		})
	}
}

impl ImageRequestProvider for ProjectSuki {
	fn get_image_request(&self, url: String, _context: Option<PageContext>) -> Result<Request> {
		request(&url)
	}
}

register_source!(
	ProjectSuki,
	ListingProvider,
	Home,
	DeepLinkHandler,
	ImageRequestProvider
);

#[cfg(test)]
mod tests {
	use super::{ProjectSuki, helpers::*};
	use aidoku::{DeepLinkHandler, DeepLinkResult, MangaStatus};
	use aidoku_test::aidoku_test;

	#[aidoku_test]
	fn parses_keys() {
		assert_eq!(manga_key_from_url("/book/207975"), Some("207975".into()));
		assert_eq!(
			chapter_keys_from_url("https://projectsuki.com/read/207975/39848/1"),
			Some(("207975".into(), "39848".into()))
		);
		assert_eq!(split_chapter_key("207975/39848"), Some(("207975", "39848")));
	}

	#[aidoku_test]
	fn parses_chapter_numbers_and_statuses() {
		assert_eq!(chapter_number("Chapter 106.5 - Notice"), Some(106.5));
		assert_eq!(chapter_number("Episode 12"), Some(12.0));
		assert_eq!(manga_status("Ongoing"), MangaStatus::Ongoing);
		assert_eq!(manga_status("Completed"), MangaStatus::Completed);
		assert_eq!(manga_status("Hiatus"), MangaStatus::Hiatus);
		assert_eq!(manga_status("Cancelled"), MangaStatus::Cancelled);
	}

	#[aidoku_test]
	fn handles_book_and_chapter_deep_links() {
		assert!(matches!(
			ProjectSuki.handle_deep_link("https://projectsuki.com/book/207975".into()),
			Ok(Some(DeepLinkResult::Manga { key })) if key == "207975"
		));
		assert!(matches!(
			ProjectSuki.handle_deep_link("https://projectsuki.com/read/207975/39848/1".into()),
			Ok(Some(DeepLinkResult::Chapter { manga_key, key }))
				if manga_key == "207975" && key == "207975/39848"
		));
	}

	#[aidoku_test]
	fn live_search_details_chapters_and_pages() {
		use aidoku::{Home, HomeComponentValue, Manga, PageContent, Source, Viewer, alloc::Vec};

		let source = ProjectSuki::new();
		let browse = source
			.get_search_manga_list(None, 1, Vec::new())
			.expect("browse failed");
		assert!(!browse.entries.is_empty());
		assert!(browse.has_next_page);

		let home = source.get_home().expect("home failed");
		assert!(matches!(
			home.components.first().map(|component| &component.value),
			Some(HomeComponentValue::MangaChapterList { entries, .. }) if !entries.is_empty()
		));

		let search = source
			.get_search_manga_list(Some("Apotheosis".into()), 1, Vec::new())
			.expect("smart search failed");
		assert!(search.entries.iter().any(|manga| manga.key == "144463"));

		let manga = source
			.get_manga_update(
				Manga {
					key: "207975".into(),
					..Default::default()
				},
				true,
				true,
			)
			.expect("manga update failed");
		assert_eq!(manga.title, "Logging 10,000 Years into the Future");
		assert!(manga.cover.is_some());
		assert!(
			manga
				.authors
				.as_ref()
				.is_some_and(|authors| !authors.is_empty())
		);
		assert!(
			manga
				.artists
				.as_ref()
				.is_some_and(|artists| !artists.is_empty())
		);
		assert!(manga.tags.as_ref().is_some_and(|tags| !tags.is_empty()));
		assert_eq!(manga.status, MangaStatus::Ongoing);
		assert_eq!(manga.viewer, Viewer::Webtoon);
		assert!(
			manga
				.description
				.as_deref()
				.is_some_and(|description| description.contains("Alternative titles:"))
		);
		let chapter = manga
			.chapters
			.as_ref()
			.and_then(|chapters| chapters.first())
			.cloned()
			.expect("no chapters returned");
		assert!(chapter.chapter_number.is_some());
		assert!(chapter.date_uploaded.is_some());
		assert_eq!(chapter.language.as_deref(), Some("en"));
		let pages = source
			.get_page_list(manga, chapter)
			.expect("page list failed");
		assert!(!pages.is_empty());
		assert!(matches!(pages[0].content, PageContent::Url(_, _)));
	}
}
