#![no_std]
use aidoku::{
	Chapter, ContentRating, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeComponent,
	HomeComponentValue, HomeLayout, ImageRequestProvider, Listing, ListingProvider, Manga,
	MangaPageResult, MangaStatus, Page, PageContent, PageContext, Result, Source, Viewer,
	alloc::{String, Vec, format, string::ToString, vec},
	helpers::uri::QueryParameters,
	imports::{
		defaults::defaults_get,
		html::{Document, Element},
		net::{Request, TimeUnit, set_rate_limit},
		std::{parse_date, send_partial_result},
	},
	prelude::*,
};

const BASE_URL: &str = "https://mangakatana.com";
const WWW_BASE_URL: &str = "https://www.mangakatana.com";
const REFERER: &str = "https://mangakatana.com/";

struct MangaKatana;

fn attr(el: &Element, name: &str) -> Option<String> {
	el.attr(name).filter(|s| !s.trim().is_empty())
}

fn path_key(url: &str) -> String {
	let path = url
		.split(['?', '#'])
		.next()
		.unwrap_or(url)
		.strip_prefix(BASE_URL)
		.or_else(|| {
			url.split(['?', '#'])
				.next()
				.unwrap_or(url)
				.strip_prefix(WWW_BASE_URL)
		})
		.unwrap_or(url);

	let path = path.trim_end_matches('/');
	if path.starts_with('/') {
		path.into()
	} else {
		format!("/{path}")
	}
}

fn absolute_url(url_or_path: &str) -> String {
	if url_or_path.starts_with("http://") || url_or_path.starts_with("https://") {
		url_or_path.into()
	} else if url_or_path.starts_with("//") {
		format!("https:{url_or_path}")
	} else if url_or_path.starts_with('/') {
		format!("{BASE_URL}{url_or_path}")
	} else {
		format!("{BASE_URL}/{url_or_path}")
	}
}

fn parse_status(status: &str) -> MangaStatus {
	let lower = status.to_ascii_lowercase();
	if lower.contains("ongoing") {
		MangaStatus::Ongoing
	} else if lower.contains("completed") {
		MangaStatus::Completed
	} else if lower.contains("cancelled") || lower.contains("canceled") {
		MangaStatus::Cancelled
	} else {
		MangaStatus::Unknown
	}
}

fn parse_content_rating(tags: &[String]) -> ContentRating {
	if tags.iter().any(|tag| {
		matches!(
			tag.to_ascii_lowercase().as_str(),
			"adult" | "erotica" | "sexual violence"
		)
	}) {
		ContentRating::NSFW
	} else if tags
		.iter()
		.any(|tag| matches!(tag.to_ascii_lowercase().as_str(), "ecchi" | "gore"))
	{
		ContentRating::Suggestive
	} else {
		ContentRating::Safe
	}
}

fn parse_viewer(tags: &[String]) -> Viewer {
	if tags.iter().any(|tag| {
		matches!(
			tag.to_ascii_lowercase().as_str(),
			"manhwa" | "manhua" | "webtoon"
		)
	}) {
		Viewer::Webtoon
	} else {
		Viewer::RightToLeft
	}
}

fn sort_value(index: i32) -> &'static str {
	match index {
		1 => "new",
		2 => "az",
		3 => "numc",
		_ => "latest",
	}
}

fn build_search_url(query: Option<&str>, page: i32, filters: Vec<FilterValue>) -> String {
	let query = query.map(str::trim).filter(|q| !q.is_empty());

	if let Some(query) = query {
		let mut search_by = String::from("book_name");
		for filter in filters {
			if let FilterValue::Select { id, value } = filter
				&& id == "search_by"
				&& !value.is_empty()
			{
				search_by = value;
			}
		}

		let mut qs = QueryParameters::new();
		qs.push("search", Some(query));
		qs.push("search_by", Some(&search_by));
		return format!("{BASE_URL}/page/{page}?{qs}");
	}

	let mut included_genres: Vec<String> = Vec::new();
	let mut excluded_genres: Vec<String> = Vec::new();
	let mut include_mode = String::from("and");
	let mut order = String::from("latest");
	let mut status = String::new();
	let mut chapters: Option<String> = None;

	for filter in filters {
		match filter {
			FilterValue::MultiSelect {
				id,
				included,
				excluded,
			} if id == "genre" => {
				included_genres = included;
				excluded_genres = excluded;
			}
			FilterValue::Select { id, value } if id == "include_mode" && !value.is_empty() => {
				include_mode = value;
			}
			FilterValue::Sort { id, index, .. } if id == "order" => {
				order = sort_value(index).into();
			}
			FilterValue::Select { id, value } if id == "status" && !value.is_empty() => {
				status = value;
			}
			FilterValue::Text { id, value } if id == "chapters" => {
				let value = value.trim();
				chapters = match value {
					"-1" => Some("e1".into()),
					"" => None,
					_ => Some(value.into()),
				};
			}
			_ => {}
		}
	}

	let mut qs = QueryParameters::new();
	qs.push("filter", Some("1"));
	if !included_genres.is_empty() {
		qs.push("include", Some(&included_genres.join("_")));
	}
	if !excluded_genres.is_empty() {
		qs.push("exclude", Some(&excluded_genres.join("_")));
	}
	qs.push("include_mode", Some(&include_mode));
	qs.push("order", Some(&order));
	if !status.is_empty() {
		qs.push("status", Some(&status));
	}
	if let Some(chapters) = &chapters {
		qs.push("chapters", Some(chapters));
	}

	format!("{BASE_URL}/manga/page/{page}?{qs}")
}

fn parse_manga_item(el: Element) -> Option<Manga> {
	let link = el
		.select_first("div.text > h3 > a")
		.or_else(|| el.select_first("h3 a"))?;
	let href = attr(&link, "abs:href").or_else(|| attr(&link, "href"))?;
	let key = path_key(&href);
	let title = link
		.own_text()
		.or_else(|| link.text())
		.unwrap_or_default()
		.trim()
		.into();
	let cover = el
		.select_first("img")
		.and_then(|img| attr(&img, "abs:src").or_else(|| attr(&img, "src")));

	Some(Manga {
		key,
		title,
		cover,
		url: Some(absolute_url(&href)),
		..Default::default()
	})
}

fn parse_manga_list(html: &Document) -> MangaPageResult {
	let entries = html
		.select("div#book_list > div.item")
		.map(|els| els.filter_map(parse_manga_item).collect())
		.unwrap_or_default();

	MangaPageResult {
		entries,
		has_next_page: html.select_first("a.next.page-numbers").is_some(),
	}
}

fn parse_single_manga_search_result(html: &Document) -> Option<MangaPageResult> {
	let title = html.select_first("h1.heading")?.text()?;
	let href = html
		.select_first("link[rel=canonical]")
		.and_then(|el| attr(&el, "abs:href").or_else(|| attr(&el, "href")))
		.or_else(|| {
			html.select_first("meta[property='og:url'], meta[name='og:url']")
				.and_then(|el| attr(&el, "content"))
		})?;
	let key = path_key(&href);
	let cover = html
		.select_first("div.media div.cover img")
		.and_then(|img| attr(&img, "abs:src").or_else(|| attr(&img, "src")));

	Some(MangaPageResult {
		entries: vec![Manga {
			key,
			title,
			cover,
			url: Some(absolute_url(&href)),
			..Default::default()
		}],
		has_next_page: false,
	})
}

fn collect_texts(html: &Document, selector: &str) -> Option<Vec<String>> {
	let values: Vec<String> = html
		.select(selector)?
		.filter_map(|el| el.text())
		.map(|s| s.trim().into())
		.filter(|s: &String| !s.is_empty())
		.collect();
	if values.is_empty() {
		None
	} else {
		Some(values)
	}
}

fn parse_chapter_number(s: &str) -> Option<f32> {
	let lower = s.to_ascii_lowercase();
	for marker in ["chapter", "ch. ", "ch.", "ch "] {
		if let Some(idx) = lower.find(marker)
			&& let Some(value) = parse_first_f32(&s[idx + marker.len()..])
		{
			return Some(value);
		}
	}

	parse_first_f32(s)
}

fn parse_first_f32(s: &str) -> Option<f32> {
	let mut value = String::new();
	let mut seen_digit = false;
	let mut seen_dot = false;

	for c in s.chars() {
		if c.is_ascii_digit() {
			value.push(c);
			seen_digit = true;
		} else if c == '.' && seen_digit && !seen_dot {
			value.push(c);
			seen_dot = true;
		} else if seen_digit {
			break;
		}
	}

	seen_digit.then(|| value.parse().ok()).flatten()
}

fn extract_image_array_name(script: &str) -> Option<String> {
	for marker in ["data-src',", "data-src\","] {
		if let Some(idx) = script.find(marker) {
			let rest = script[idx + marker.len()..].trim_start();
			let name: String = rest
				.chars()
				.take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
				.collect();
			if !name.is_empty() {
				return Some(name);
			}
		}
	}
	None
}

fn extract_array_literal<'a>(script: &'a str, array_name: &str) -> Option<&'a str> {
	let needle = format!("var {array_name}");
	let start = script.find(&needle)?;
	let after_var = &script[start + needle.len()..];
	let eq_idx = after_var.find('=')?;
	let after_eq = after_var[eq_idx + 1..].trim_start();
	let after_open = after_eq.strip_prefix('[')?;
	let close_idx = after_open.find(']')?;
	Some(&after_open[..close_idx])
}

fn extract_single_quoted_strings(input: &str) -> Vec<String> {
	let mut values = Vec::new();
	let mut rest = input;

	while let Some(start) = rest.find('\'') {
		rest = &rest[start + 1..];
		let Some(end) = rest.find('\'') else {
			break;
		};
		let value = &rest[..end];
		if !value.is_empty() {
			values.push(value.into());
		}
		rest = &rest[end + 1..];
	}

	values
}

fn extract_page_urls_from_script(script: &str) -> Vec<String> {
	extract_image_array_name(script)
		.and_then(|name| extract_array_literal(script, &name).map(extract_single_quoted_strings))
		.unwrap_or_default()
}

fn chapter_url(chapter: &Chapter) -> String {
	let mut url = absolute_url(&chapter.key);
	let server = defaults_get::<String>("serverPreference").unwrap_or_default();
	let server = server.trim();
	if !server.is_empty() {
		let mut qs = QueryParameters::new();
		qs.push("sv", Some(server));
		url.push('?');
		url.push_str(&qs.to_string());
	}
	url
}

impl Source for MangaKatana {
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
		let url = build_search_url(query.as_deref(), page, filters);
		let html = Request::get(&url)?.html()?;
		let result = parse_manga_list(&html);

		if result.entries.is_empty()
			&& let Some(single_result) = parse_single_manga_search_result(&html)
		{
			return Ok(single_result);
		}

		Ok(result)
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let url = absolute_url(&manga.key);
		let html = Request::get(&url)?.html()?;

		if needs_details {
			manga.title = html
				.select_first("h1.heading")
				.and_then(|el| el.text())
				.unwrap_or(manga.title);
			manga.cover = html
				.select_first("div.media div.cover img")
				.and_then(|img| attr(&img, "abs:src").or_else(|| attr(&img, "src")));
			manga.authors = collect_texts(&html, ".author");
			manga.description = {
				let summary: String = html
					.select(".summary > p")
					.map(|els| {
						els.filter_map(|el| el.text())
							.map(|s| s.trim().into())
							.filter(|s: &String| !s.is_empty())
							.collect::<Vec<String>>()
							.join("\n")
					})
					.unwrap_or_default();
				let alt_names: String = html
					.select_first(".alt_name")
					.and_then(|el| el.text())
					.map(|s| s.trim().into())
					.unwrap_or_default();

				if summary.is_empty() && alt_names.is_empty() {
					None
				} else if alt_names.is_empty() {
					Some(summary)
				} else if summary.is_empty() {
					Some(format!("Alt name(s): {alt_names}"))
				} else {
					Some(format!("{summary}\n\nAlt name(s): {alt_names}"))
				}
			};
			let tags = collect_texts(&html, ".genres > a").unwrap_or_default();
			manga.status = html
				.select_first(".value.status")
				.and_then(|el| el.text())
				.map(|status| parse_status(&status))
				.unwrap_or(MangaStatus::Unknown);
			manga.content_rating = parse_content_rating(&tags);
			manga.viewer = parse_viewer(&tags);
			if !tags.is_empty() {
				manga.tags = Some(tags);
			}
			manga.url = Some(url);

			if needs_chapters {
				send_partial_result(&manga);
			}
		}

		if needs_chapters {
			manga.chapters = html.select("tr:has(.chapter)").map(|els| {
				els.filter_map(|el| {
					let link = el.select_first("a")?;
					let href = attr(&link, "abs:href").or_else(|| attr(&link, "href"))?;
					let title = link.text().unwrap_or_default();
					let date_uploaded = el
						.select_first(".update_time")
						.and_then(|date| date.text())
						.and_then(|date| parse_date(date.trim(), "MMM-dd-yyyy"));

					Some(Chapter {
						key: path_key(&href),
						title: Some(title.clone()),
						chapter_number: parse_chapter_number(&title),
						date_uploaded,
						url: Some(absolute_url(&href)),
						..Default::default()
					})
				})
				.collect()
			});
		}

		Ok(manga)
	}

	fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let url = chapter_url(&chapter);
		let html = Request::get(&url)?.html()?;

		let page_urls = html
			.select("script")
			.map(|scripts| {
				scripts
					.filter_map(|script| {
						let data = script.data()?;
						if !data.contains("data-src") {
							return None;
						}
						let urls = extract_page_urls_from_script(&data);
						(!urls.is_empty()).then_some(urls)
					})
					.next()
					.unwrap_or_default()
			})
			.unwrap_or_default();

		Ok(page_urls
			.into_iter()
			.map(|url| Page {
				content: PageContent::url(absolute_url(&url)),
				..Default::default()
			})
			.collect())
	}
}

impl ListingProvider for MangaKatana {
	fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
		let url = match listing.id.as_str() {
			"latest" => format!("{BASE_URL}/page/{page}"),
			"popular" => format!("{BASE_URL}/manga/page/{page}"),
			_ => bail!("Unknown listing: {}", listing.id),
		};
		Ok(parse_manga_list(&Request::get(url)?.html()?))
	}
}

impl Home for MangaKatana {
	fn get_home(&self) -> Result<HomeLayout> {
		let html = Request::get(format!("{BASE_URL}/page/1"))?.html()?;
		let latest = parse_manga_list(&html).entries;

		Ok(HomeLayout {
			components: vec![HomeComponent {
				title: Some("Latest Updates".into()),
				subtitle: None,
				value: HomeComponentValue::Scroller {
					entries: latest.into_iter().map(Into::into).collect(),
					listing: None,
				},
			}],
		})
	}
}

impl DeepLinkHandler for MangaKatana {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		let key = path_key(&url);
		if !key.starts_with("/manga/") {
			return Ok(None);
		}

		let parts: Vec<&str> = key.trim_start_matches('/').split('/').collect();
		if parts.len() > 2 {
			let manga_key = format!("/{}/{}", parts[0], parts[1]);
			Ok(Some(DeepLinkResult::Chapter { manga_key, key }))
		} else {
			Ok(Some(DeepLinkResult::Manga { key }))
		}
	}
}

impl ImageRequestProvider for MangaKatana {
	fn get_image_request(&self, url: String, _context: Option<PageContext>) -> Result<Request> {
		Ok(Request::get(url)?.header("Referer", REFERER))
	}
}

register_source!(
	MangaKatana,
	ListingProvider,
	Home,
	DeepLinkHandler,
	ImageRequestProvider
);

#[cfg(test)]
mod tests {
	use super::*;
	use aidoku::{FilterValue, alloc::vec};
	use aidoku_test::aidoku_test;

	#[aidoku_test]
	fn search_url_uses_text_search_type_when_query_is_present() {
		let url = build_search_url(
			Some("one piece"),
			2,
			vec![FilterValue::Select {
				id: "search_by".into(),
				value: "author".into(),
			}],
		);

		assert_eq!(
			url,
			"https://mangakatana.com/page/2?search=one%20piece&search_by=author"
		);
	}

	#[aidoku_test]
	fn browse_url_translates_filters_to_mangakatana_query_parameters() {
		let url = build_search_url(
			None,
			3,
			vec![
				FilterValue::MultiSelect {
					id: "genre".into(),
					included: vec!["action".into(), "comedy".into()],
					excluded: vec!["adult".into()],
				},
				FilterValue::Select {
					id: "include_mode".into(),
					value: "or".into(),
				},
				FilterValue::Sort {
					id: "order".into(),
					index: 3,
					ascending: false,
				},
				FilterValue::Select {
					id: "status".into(),
					value: "2".into(),
				},
				FilterValue::Text {
					id: "chapters".into(),
					value: "-1".into(),
				},
			],
		);

		assert_eq!(
			url,
			"https://mangakatana.com/manga/page/3?filter=1&include=action_comedy&exclude=adult&include_mode=or&order=numc&status=2&chapters=e1"
		);
	}

	#[aidoku_test]
	fn browse_url_omits_chapters_when_chapter_filter_is_blank() {
		let url = build_search_url(
			None,
			1,
			vec![FilterValue::Text {
				id: "chapters".into(),
				value: "".into(),
			}],
		);

		assert_eq!(
			url,
			"https://mangakatana.com/manga/page/1?filter=1&include_mode=and&order=latest"
		);
	}

	#[aidoku_test]
	fn chapter_number_prefers_chapter_marker_over_volume_number() {
		assert_eq!(parse_chapter_number("Vol.5 Ch.12: The Turn"), Some(12.0));
		assert_eq!(parse_chapter_number("Volume 2 Chapter 7.5"), Some(7.5));
		assert_eq!(parse_chapter_number("Special 3"), Some(3.0));
	}

	#[aidoku_test]
	fn page_script_extraction_finds_urls_from_named_data_src_array() {
		let script = r#"
			var ignored=['https://example.invalid/a.jpg'];
			initReader("data-src", pages);
			var pages=['https://cdn.example/001.jpg','https://cdn.example/002.png'];
		"#;

		assert_eq!(
			extract_page_urls_from_script(script),
			vec![
				String::from("https://cdn.example/001.jpg"),
				String::from("https://cdn.example/002.png")
			]
		);
	}

	#[aidoku_test]
	fn path_key_strips_known_mangakatana_hosts() {
		assert_eq!(
			path_key("https://mangakatana.com/manga/title.123/c1"),
			"/manga/title.123/c1"
		);
		assert_eq!(path_key("/manga/title.123"), "/manga/title.123");
	}
}
