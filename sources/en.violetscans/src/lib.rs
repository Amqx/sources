#![no_std]
use aidoku::{
	Chapter, ContentRating, DeepLinkHandler, DeepLinkResult, FilterValue, ImageRequestProvider,
	Manga, MangaPageResult, MangaStatus, Page, PageContent, PageContext, Result, Source, Viewer,
	alloc::{String, Vec, format, string::ToString, vec::Vec as AllocVec},
	helpers::{string::StripPrefixOrSelf, uri::QueryParameters},
	imports::{
		error::AidokuError,
		html::Element,
		net::Request,
		std::{parse_date, send_partial_result},
	},
	prelude::*,
};

const BASE_URL: &str = "https://violetscans.org";
const MANGA_PATH: &str = "/comics";

struct VioletScans;

fn img_attr(el: &Element) -> Option<String> {
	el.attr("abs:data-lazy-src")
		.or_else(|| el.attr("abs:data-src"))
		.or_else(|| el.attr("abs:src"))
}

fn key_from_url(url: &str) -> String {
	url.strip_prefix_or_self(BASE_URL).into()
}

fn imptdt_value(html: &Element, label: &str) -> Option<String> {
	let label_lower = label.to_lowercase();
	html.select(".tsinfo .imptdt")?.find_map(|el| {
		let text = el.text()?.to_lowercase();
		if !text.contains(&label_lower) {
			return None;
		}
		let value = el
			.select_first("i")
			.and_then(|i| i.own_text().or_else(|| i.text()))?;
		let trimmed = value.trim();
		if trimmed.is_empty()
			|| trimmed == "-"
			|| trimmed.eq_ignore_ascii_case("n/a")
			|| trimmed.eq_ignore_ascii_case("unknown")
		{
			None
		} else {
			Some(trimmed.into())
		}
	})
}

fn parse_status(s: &str) -> MangaStatus {
	match s.trim().to_lowercase().as_str() {
		"ongoing" | "on going" | "publishing" | "updating" => MangaStatus::Ongoing,
		"completed" | "finished" | "one-shot" => MangaStatus::Completed,
		"hiatus" | "on hold" | "paused" => MangaStatus::Hiatus,
		"cancelled" | "canceled" | "dropped" | "discontinued" => MangaStatus::Cancelled,
		_ => MangaStatus::Unknown,
	}
}

fn parse_viewer(s: &str) -> Viewer {
	match s.trim().to_lowercase().as_str() {
		"manga" | "one-shot" | "oneshot" | "doujinshi" => Viewer::RightToLeft,
		"manhwa" | "manhua" | "webtoon" | "webtoons" => Viewer::Webtoon,
		"comic" => Viewer::LeftToRight,
		_ => Viewer::Unknown,
	}
}

fn find_first_f32(s: &str) -> Option<f32> {
	let mut num = String::new();
	let mut found_digit = false;
	let mut dot_found = false;
	for c in s.chars() {
		if c.is_ascii_digit() {
			num.push(c);
			found_digit = true;
		} else if c == '.' && found_digit && !dot_found {
			num.push(c);
			dot_found = true;
		} else if found_digit {
			break;
		}
	}
	if found_digit { num.parse().ok() } else { None }
}

fn extract_images(content: &str) -> AllocVec<String> {
	let needle = "\"images\":[";
	let Some(start) = content.find(needle) else {
		return AllocVec::new();
	};
	let after = &content[start + needle.len() - 1..];
	let Some(end) = after.find(']') else {
		return AllocVec::new();
	};
	serde_json::from_str::<AllocVec<String>>(&after[..=end]).unwrap_or_default()
}

fn normalize_ws(s: &str) -> String {
	let parts: AllocVec<&str> = s.split_whitespace().collect();
	parts.join(" ")
}

impl Source for VioletScans {
	fn new() -> Self {
		Self
	}

	fn get_search_manga_list(
		&self,
		query: Option<String>,
		page: i32,
		filters: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		let mut qs = QueryParameters::new();
		qs.push("page", Some(&page.to_string()));
		if let Some(q) = query.as_deref() {
			qs.push("title", Some(q));
		}

		for filter in filters {
			match filter {
				FilterValue::Text { id, value } => qs.push(&id, Some(&value)),
				FilterValue::Select { id, value } if !value.is_empty() => {
					qs.set(&id, Some(&value));
				}
				FilterValue::Sort { id, index, .. } => {
					let value = match index {
						1 => "title",
						2 => "titlereverse",
						3 => "update",
						4 => "latest",
						5 => "popular",
						_ => "",
					};
					if !value.is_empty() {
						qs.set(&id, Some(value));
					}
				}
				FilterValue::MultiSelect {
					id,
					included,
					excluded,
				} => {
					for item in included {
						qs.push(&id, Some(&item));
					}
					for item in excluded {
						qs.push(&id, Some(&format!("-{item}")));
					}
				}
				_ => {}
			}
		}

		let url = format!("{BASE_URL}{MANGA_PATH}/?{qs}");
		let html = Request::get(&url)?.html()?;

		let entries: Vec<Manga> = html
			.select(".listupd .bsx, .utao .uta .imgu")
			.map(|els| {
				els.filter_map(|el| {
					let link = el.select_first("a")?;
					let href = link.attr("abs:href")?;
					Some(Manga {
						key: key_from_url(&href),
						title: link.attr("title").unwrap_or_default(),
						cover: el.select_first("img").and_then(|img| img_attr(&img)),
						..Default::default()
					})
				})
				.collect()
			})
			.unwrap_or_default();

		let has_next_page = html
			.select_first("div.pagination .next, div.hpage .r")
			.is_some();

		Ok(MangaPageResult {
			entries,
			has_next_page,
		})
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let manga_url = format!("{BASE_URL}{}", manga.key);
		let html = Request::get(&manga_url)?.html()?;

		if needs_details {
			let details = html
				.select_first(".main-info, div.bigcontent, div.animefull, div.postbody")
				.ok_or_else(|| error!("Unable to find manga details"))?;

			if let Some(title) = details
				.select_first("h1.entry-title")
				.and_then(|el| el.text())
			{
				manga.title = title.trim().into();
			}
			manga.cover = details
				.select_first(".thumb img, .bigcover img, .infomanga > div[itemprop=image] img")
				.and_then(|img| img_attr(&img))
				.or(manga.cover);

			if let Some(author) = imptdt_value(&details, "Author") {
				manga.authors = Some(author.split(',').map(|s| s.trim().into()).collect());
			}
			if let Some(artist) = imptdt_value(&details, "Artist") {
				manga.artists = Some(artist.split(',').map(|s| s.trim().into()).collect());
			}

			let description = details
				.select_first(".entry-content[itemprop=description], .desc, .entry-content")
				.and_then(|el| el.text())
				.map(|s| s.trim().into());
			let alt_name: Option<String> = details
				.select_first(".alternative .desktop-titles, .alternative")
				.and_then(|el| el.text())
				.and_then(|s| {
					let t = s.trim();
					if t.is_empty() { None } else { Some(t.into()) }
				});
			manga.description = match (description, alt_name) {
				(Some(d), Some(a)) => Some(format!("{d}\n\nAlternative Names: {a}")),
				(Some(d), None) => Some(d),
				(None, Some(a)) => Some(format!("Alternative Names: {a}")),
				(None, None) => None,
			};

			manga.url = Some(manga_url.clone());

			let mut tags: Vec<String> = details
				.select(".mgen a, div.gnr a, .seriestugenre a")
				.map(|els| {
					els.filter_map(|el| el.text())
						.map(|s| s.trim().into())
						.collect()
				})
				.unwrap_or_default();

			let series_type = imptdt_value(&details, "Type");
			if let Some(t) = &series_type
				&& !tags.iter().any(|x| x.eq_ignore_ascii_case(t))
			{
				tags.push(t.clone());
			}

			let is_nsfw = tags.iter().any(|t| {
				let l = t.to_lowercase();
				l == "mature" || l == "smut" || l == "adult" || l == "yaoi"
			});
			let is_suggestive = tags.iter().any(|t| {
				let l = t.to_lowercase();
				l == "ecchi" || l == "shounen ai" || l == "shoujo ai"
			});
			manga.tags = if tags.is_empty() { None } else { Some(tags) };

			manga.status = imptdt_value(&details, "Status")
				.map(|s| parse_status(&s))
				.unwrap_or(MangaStatus::Unknown);

			manga.content_rating = if is_nsfw {
				ContentRating::NSFW
			} else if is_suggestive {
				ContentRating::Suggestive
			} else {
				ContentRating::Safe
			};

			manga.viewer = series_type
				.as_deref()
				.map(parse_viewer)
				.unwrap_or(Viewer::Unknown);

			send_partial_result(&manga);
		}

		if needs_chapters {
			manga.chapters = html.select("#chapterlist li:not(:has(svg))").map(|els| {
				els.filter_map(|el| {
					let link = el.select_first("a")?;
					let href = link.attr("abs:href")?;
					if href.is_empty() || href.starts_with('#') {
						return None;
					}
					let raw_title = el
						.select_first(".chapternum")
						.and_then(|e| e.text())
						.or_else(|| link.text())
						.unwrap_or_default();
					let title = normalize_ws(&raw_title);
					let chapter_number = find_first_f32(&title);
					let date_uploaded = el
						.select_first(".chapterdate")
						.and_then(|e| e.text())
						.and_then(|s| parse_date(s.trim(), "MMMM d, yyyy"));
					let display_title = match chapter_number {
						Some(n) => {
							let int_form = format!("Chapter {}", n as i32);
							let float_form = format!("Chapter {n}");
							if title == int_form || title == float_form {
								None
							} else {
								Some(title)
							}
						}
						None => Some(title),
					};
					Some(Chapter {
						key: key_from_url(&href),
						title: display_title,
						chapter_number,
						date_uploaded,
						url: Some(href),
						..Default::default()
					})
				})
				.collect()
			});
		}

		Ok(manga)
	}

	fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let url = format!("{BASE_URL}{}", chapter.key);
		let body = Request::get(&url)?.string()?;

		let images = extract_images(&body);
		if images.is_empty() {
			return Err(AidokuError::message("No pages found!"));
		}

		Ok(images
			.into_iter()
			.map(|u| Page {
				content: PageContent::url(u),
				..Default::default()
			})
			.collect())
	}
}

impl ImageRequestProvider for VioletScans {
	fn get_image_request(&self, url: String, _ctx: Option<PageContext>) -> Result<Request> {
		Ok(Request::get(url)?
			.header("Accept", "image/avif,image/webp,image/png,image/jpeg,*/*")
			.header("Referer", &format!("{BASE_URL}/")))
	}
}

impl DeepLinkHandler for VioletScans {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		let Some(path) = url.strip_prefix(BASE_URL) else {
			return Ok(None);
		};
		if path.starts_with(MANGA_PATH) {
			Ok(Some(DeepLinkResult::Manga { key: path.into() }))
		} else {
			Ok(None)
		}
	}
}

register_source!(VioletScans, ImageRequestProvider, DeepLinkHandler);
