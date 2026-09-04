use aidoku::{
	Chapter, ContentRating, Manga, MangaStatus, Viewer,
	alloc::{String, Vec, string::ToString},
	imports::html::{Document, Element, ElementList},
	prelude::*,
};

use crate::helpers::*;

/// The type badges the site puts on a title, and the viewer each implies.
const TYPES: [(&str, Viewer); 7] = [
	("manga", Viewer::RightToLeft),
	("manhwa", Viewer::Webtoon),
	("manhua", Viewer::Webtoon),
	("shounen", Viewer::RightToLeft),
	("seinen", Viewer::RightToLeft),
	("shoujo", Viewer::RightToLeft),
	("josei", Viewer::RightToLeft),
];

pub fn parse_manga_list(doc: &Document) -> Vec<Manga> {
	doc.select("div.relative.group")
		.map(|entries| entries.filter_map(|entry| parse_entry(&entry)).collect())
		.unwrap_or_default()
}

fn parse_entry(entry: &Element) -> Option<Manga> {
	let link = entry.select_first("a[href*=\"/manga/\"]")?;
	let key = manga_slug(&link.attr("href")?)?;

	let title = entry
		.select_first("div[data-flux-heading], h3, h4")
		.and_then(|el| el.text())
		.or_else(|| link.attr("title"))
		.or_else(|| link.text())
		.filter(|title| !title.is_empty())?;

	Some(Manga {
		cover: cover(entry),
		content_rating: content_rating(entry.select("span")),
		url: Some(manga_url(&key)),
		key,
		title,
		..Default::default()
	})
}

/// The first usable `<img>`, preferring one that carries an `alt`
fn cover(element: &Element) -> Option<String> {
	let images = element.select("img")?;
	let mut fallback = None;
	for image in images {
		let Some(url) = image_url(&image) else {
			continue;
		};
		if image.attr("alt").is_some_and(|alt| !alt.trim().is_empty()) {
			return Some(url);
		}
		fallback.get_or_insert(url);
	}
	fallback
}

/// Titles the site gates behind an "18+" overlay.
fn content_rating(spans: Option<ElementList>) -> ContentRating {
	let is_adult = spans.is_some_and(|spans| {
		spans
			.into_iter()
			.any(|span| span.own_text().is_some_and(|text| text.contains("18+")))
	});
	if is_adult {
		ContentRating::NSFW
	} else {
		ContentRating::Unknown
	}
}

pub fn parse_details(doc: &Document, key: String) -> Option<Manga> {
	let title = doc
		.select_first("h1")
		.or_else(|| doc.select_first("[data-flux-heading]"))
		.and_then(|el| el.text())
		.filter(|title| !title.is_empty())?;

	let badges = badge_texts(doc);

	let (tags, viewer) = tags_and_viewer(doc, &badges);

	Some(Manga {
		key: key.clone(),
		title,
		cover: doc
			.select_first(".w-32 picture img")
			.or_else(|| doc.select_first(".w-32 img"))
			.as_ref()
			.and_then(image_url),
		authors: links_text(doc, "a[href*=\"/author/\"]"),
		description: description(doc),
		url: Some(manga_url(&key)),
		tags,
		status: status(&badges),
		content_rating: details_content_rating(doc, &badges),
		viewer,
		..Default::default()
	})
}

fn details_content_rating(doc: &Document, badges: &[String]) -> ContentRating {
	if badges.iter().any(|badge| badge.contains("18+")) {
		return ContentRating::NSFW;
	}
	content_rating(
		doc.select_first(".w-32")
			.and_then(|cover| cover.select("span")),
	)
}

/// The badge row under the title, which carries the status, origin and type.
fn badge_texts(doc: &Document) -> Vec<String> {
	doc.select_first("div.flex.items-center.gap-2.justify-center.mb-2")
		.and_then(|row| row.select("div[data-flux-badge], span"))
		.map(|badges| {
			badges
				.filter_map(|badge| badge.own_text())
				.map(|text| text.trim().to_string())
				.filter(|text| !text.is_empty())
				.collect()
		})
		.unwrap_or_default()
}

fn tags_and_viewer(doc: &Document, badges: &[String]) -> (Option<Vec<String>>, Viewer) {
	let mut tags = Vec::new();
	let mut viewer = Viewer::Unknown;

	for badge in badges {
		let lowercased = badge.to_lowercase();
		if let Some((name, badge_viewer)) = TYPES.iter().find(|(name, _)| *name == lowercased) {
			if viewer == Viewer::Unknown {
				viewer = *badge_viewer;
			}
			let mut capitalized = name[..1].to_uppercase();
			capitalized.push_str(&name[1..]);
			tags.push(capitalized);
		}
	}

	if let Some(genres) = links_text(doc, "a[href*=\"/genre/\"]") {
		tags.extend(genres);
	}

	(if tags.is_empty() { None } else { Some(tags) }, viewer)
}

fn links_text(doc: &Document, selector: &str) -> Option<Vec<String>> {
	let values: Vec<String> = doc
		.select(selector)?
		.filter_map(|link| link.text())
		.map(|text| text.trim().to_string())
		.filter(|text| !text.is_empty())
		.collect();
	(!values.is_empty()).then_some(values)
}

fn status(badges: &[String]) -> MangaStatus {
	for badge in badges {
		let badge = badge.to_lowercase();
		if badge.contains("ongoing") || badge.contains("releasing") {
			return MangaStatus::Ongoing;
		}
		if badge.contains("completed") {
			return MangaStatus::Completed;
		}
		if badge.contains("hiatus") {
			return MangaStatus::Hiatus;
		}
		if badge.contains("cancelled") || badge.contains("dropped") {
			return MangaStatus::Cancelled;
		}
	}
	MangaStatus::Unknown
}

fn description(doc: &Document) -> Option<String> {
	let mut description = doc
		.select_first("p.leading-relaxed")
		.and_then(|el| el.text())
		.map(|text| text.trim().to_string())
		.unwrap_or_default();

	// The alternative titles sit in a `·`-separated paragraph whose only
	// distinguishing mark is an arbitrary-value Tailwind class.
	let alternatives = doc
		.select("p")
		.into_iter()
		.flatten()
		.filter(|p| {
			p.attr("class")
				.is_some_and(|class| class.contains("text-[13px]"))
		})
		.find_map(|p| p.text())
		.map(|text| split_details(&text))
		.unwrap_or_default();

	if !alternatives.is_empty() {
		if !description.is_empty() {
			description.push_str("\n\n");
		}
		description.push_str("**Alternative Titles:**\n");
		for alternative in alternatives {
			description.push_str(&format!("- {alternative}\n"));
		}
	}

	(!description.is_empty()).then_some(description)
}

/// Parse one language's chapter rows out of a `manga.chapter-list` fragment.
pub fn parse_chapters(doc: &Document, language: &str) -> Vec<Chapter> {
	let mut chapters = Vec::new();

	if let Some(rows) = doc.select("a.gap-4") {
		for row in rows {
			let Some(heading) = row.select_first("div[data-flux-heading]") else {
				continue;
			};
			let Some(key) = read_key(&row) else {
				continue;
			};
			let (number, date) = row_meta(&row, &heading);
			chapters.push(chapter(key, number, date, None, language));
		}
	}

	if let Some(dropdowns) = doc.select("ui-dropdown") {
		for dropdown in dropdowns {
			let Some(button) = dropdown.select_first("button") else {
				continue;
			};
			let Some(heading) = button.select_first("div[data-flux-heading]") else {
				continue;
			};
			let (number, date) = row_meta(&button, &heading);

			let Some(items) = dropdown.select("ui-menu a[data-flux-menu-item]") else {
				continue;
			};
			let mut unknown = 0;
			for item in items {
				let Some(key) = read_key(&item) else {
					continue;
				};
				let group = item
					.select_first("span.text-sm")
					.and_then(|el| el.text())
					.map(|text| text.trim().to_string())
					.filter(|text| !text.is_empty() && !text.eq_ignore_ascii_case("Unknown group"));
				let group = group.unwrap_or_else(|| {
					unknown += 1;
					format!("Unknown {unknown}")
				});
				chapters.push(chapter(key, number, date, Some(group), language));
			}
		}
	}

	chapters
}

fn read_key(link: &Element) -> Option<String> {
	let href = link.attr("href")?;
	href.contains("/read/").then(|| to_path(&href))
}

/// The chapter number from the row's heading, and the upload date from the
/// `·`-separated detail line beneath it.
fn row_meta(row: &Element, heading: &Element) -> (Option<f32>, Option<i64>) {
	let number = heading
		.text()
		.filter(|text| !text.trim().is_empty())
		.or_else(|| row.select_first("div.w-10").and_then(|el| el.text()))
		.and_then(|text| parse_chapter_number(&text));

	let date = row
		.select_first("p[data-flux-text]")
		.and_then(|el| el.text())
		.map(|text| split_details(&text))
		.unwrap_or_default()
		.into_iter()
		.find_map(|part| parse_relative_date(&part));

	(number, date)
}

fn chapter(
	key: String,
	chapter_number: Option<f32>,
	date_uploaded: Option<i64>,
	group: Option<String>,
	language: &str,
) -> Chapter {
	Chapter {
		url: Some(chapter_url(&key)),
		key,
		chapter_number,
		date_uploaded,
		scanlators: group.map(|group| Vec::from([group])),
		language: Some(language.into()),
		..Default::default()
	}
}

/// Newest first, which is the order Aidoku expects a chapter list in.
pub fn sort_chapters(chapters: &mut [Chapter]) {
	chapters.sort_by(|a, b| {
		b.chapter_number
			.unwrap_or_default()
			.total_cmp(&a.chapter_number.unwrap_or_default())
	});
}
