use crate::{BASE_URL, MangaBall};
use aidoku::{
	ContentRating, Manga, MangaStatus, Result, Viewer,
	alloc::{String, Vec, format, vec},
	imports::{
		defaults::defaults_get,
		html::Document,
		net::{Request, Response},
	},
	prelude::*,
};
use serde::de::DeserializeOwned;

const ADULT_COOKIE: &str = "show18PlusContent=true";
const USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) \
	AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1";

pub fn get_request(url: &str) -> Result<Request> {
	Ok(Request::get(url)?
		.header("User-Agent", USER_AGENT)
		.header(
			"Accept",
			"text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
		)
		.header("Accept-Language", "en-US,en;q=0.9")
		.header("Referer", &format!("{BASE_URL}/"))
		.header("Cookie", ADULT_COOKIE))
}

impl MangaBall {
	fn csrf_token(&self, refresh: bool) -> Result<String> {
		if !refresh && let Some(token) = self.csrf.borrow().as_ref() {
			return Ok(token.clone());
		}
		let html = get_request(BASE_URL)?.html()?;
		let token = html
			.select_first("meta[name=csrf-token]")
			.and_then(|meta| meta.attr("content"))
			.filter(|token| !token.is_empty())
			.ok_or_else(|| error!("CSRF token not found"))?;
		*self.csrf.borrow_mut() = Some(token.clone());
		Ok(token)
	}

	pub fn remember_token(&self, html: &Document) {
		if let Some(token) = html
			.select_first("meta[name=csrf-token]")
			.and_then(|meta| meta.attr("content"))
		{
			*self.csrf.borrow_mut() = Some(token);
		}
	}

	fn post_form(&self, path: &str, body: &str) -> Result<Response> {
		for attempt in 0..2 {
			let token = self.csrf_token(attempt > 0)?;
			let response = Request::post(format!("{BASE_URL}{path}"))?
				.header("User-Agent", USER_AGENT)
				.header("Accept", "application/json, text/javascript, */*; q=0.01")
				.header("Accept-Language", "en-US,en;q=0.9")
				.header("Content-Type", "application/x-www-form-urlencoded")
				.header("Cookie", ADULT_COOKIE)
				.header("Origin", BASE_URL)
				.header("Referer", &format!("{BASE_URL}/"))
				.header("X-Requested-With", "XMLHttpRequest")
				.header("X-CSRF-TOKEN", &token)
				.body(body)
				.send()?;
			if response.status_code() == 403 && attempt == 0 {
				*self.csrf.borrow_mut() = None;
				continue;
			}
			if response.status_code() >= 400 {
				bail!("Response Error: {}", response.status_code())
			}
			return Ok(response);
		}
		bail!("Unable to establish a MangaBall session")
	}

	pub fn post_json<T: DeserializeOwned>(&self, path: &str, body: &str) -> Result<T> {
		self.post_form(path, body)?.get_json_owned()
	}
}

pub fn selected_languages() -> Vec<String> {
	defaults_get::<Vec<String>>("languages")
		.filter(|languages| !languages.is_empty())
		.unwrap_or_else(|| vec!["en".into()])
		.into_iter()
		.flat_map(|language| {
			api_languages(&language)
				.iter()
				.map(|value| (*value).into())
				.collect::<Vec<String>>()
		})
		.collect()
}

pub fn api_languages(language: &str) -> &'static [&'static str] {
	match language {
		"ca" => &["ca", "ca-ad", "ca-es", "ca-fr", "ca-it", "ca-pt"],
		"es" => &["es", "es-ar", "es-mx", "es-es", "es-la", "es-419"],
		"it" => &["it", "it-it"],
		"is" => &["ib", "ib-is", "is"],
		"ja" => &["jp"],
		"ko" => &["kr"],
		"kn" => &["kn", "kn-in", "kn-my", "kn-sg", "kn-tw"],
		"ml" => &["ml", "ml-in", "ml-my", "ml-sg", "ml-tw"],
		"nl" => &["nl", "nl-be"],
		"pt-BR" => &["pt-br", "pt-pt"],
		"sr" => &["sr", "sr-cyrl"],
		"th" => &["th", "th-hk", "th-kh", "th-la", "th-my", "th-sg"],
		"zh" => &["zh", "zh-cn", "zh-hk", "zh-mo", "zh-sg", "zh-tw"],
		"ar" => &["ar"],
		"bg" => &["bg"],
		"bn" => &["bn"],
		"cs" => &["cs"],
		"da" => &["da"],
		"de" => &["de"],
		"el" => &["el"],
		"en" => &["en"],
		"fa" => &["fa"],
		"fi" => &["fi"],
		"fr" => &["fr"],
		"he" => &["he"],
		"hi" => &["hi"],
		"hu" => &["hu"],
		"id" => &["id"],
		"ms" => &["ms"],
		"ne" => &["ne"],
		"no" => &["no"],
		"pl" => &["pl"],
		"ro" => &["ro"],
		"ru" => &["ru"],
		"sk" => &["sk"],
		"sl" => &["sl"],
		"sq" => &["sq"],
		"sv" => &["sv"],
		"ta" => &["ta"],
		"tr" => &["tr"],
		"uk" => &["uk"],
		"vi" => &["vi"],
		_ => &["en"],
	}
}

pub fn fill_details(html: &Document, manga: &mut Manga) -> Result<()> {
	manga.title = html
		.select_first("#comicDetail h6")
		.and_then(|element| element.own_text())
		.ok_or_else(|| error!("Manga title not found"))?;
	manga.cover = html
		.select_first("img.featured-cover")
		.and_then(|image| image.attr("abs:src"));
	manga.url = Some(crate::manga_url(&manga.key));

	let mut tags = Vec::new();
	if let Some(flag) = html
		.select_first("#featuredComicsCarousel img[src*='/flags/']")
		.and_then(|image| image.attr("src"))
	{
		let kind = if flag.contains("jp") {
			Some("Manga")
		} else if flag.contains("kr") {
			Some("Manhwa")
		} else if flag.contains("cn") {
			Some("Manhua")
		} else {
			None
		};
		if let Some(kind) = kind {
			tags.push(kind.into());
		}
	}
	if let Some(elements) = html.select("#comicDetail span[data-tag-id]") {
		tags.extend(elements.filter_map(|element| element.own_text()));
	}
	manga.tags = (!tags.is_empty()).then_some(tags);
	manga.authors = html
		.select("#comicDetail span[data-person-id]")
		.map(|elements| elements.filter_map(|element| element.own_text()).collect());
	manga.description = build_description(html);
	manga.status = html
		.select_first("span.badge-status")
		.and_then(|element| element.text())
		.map_or(MangaStatus::Unknown, |status| parse_status(&status));
	manga.viewer = parse_viewer(manga.tags.as_deref().unwrap_or_default());
	manga.content_rating = parse_content_rating(manga.tags.as_deref().unwrap_or_default());
	Ok(())
}

fn build_description(html: &Document) -> Option<String> {
	let mut sections = Vec::new();
	if let Some(description) = html
		.select_first("#descriptionContent p")
		.and_then(|element| element.text())
		.filter(|text| !text.trim().is_empty())
	{
		sections.push(description.trim().into());
	}
	if let Some(published) = html
		.select_first("#comicDetail span.badge:contains(Published)")
		.and_then(|element| element.text())
	{
		sections.push(published);
	}
	if let Some(names) = html
		.select("div.alternate-name-container")
		.map(|elements| {
			elements
				.filter_map(|element| element.text())
				.collect::<Vec<_>>()
		})
		.filter(|names| !names.is_empty())
	{
		sections.push(format!("Alternative Names:\n- {}", names.join("\n- ")));
	}
	(!sections.is_empty()).then(|| sections.join("\n\n"))
}

fn parse_status(status: &str) -> MangaStatus {
	match status.trim() {
		"Ongoing" => MangaStatus::Ongoing,
		"Completed" => MangaStatus::Completed,
		"Hiatus" | "On Hold" => MangaStatus::Hiatus,
		"Cancelled" => MangaStatus::Cancelled,
		_ => MangaStatus::Unknown,
	}
}

fn parse_viewer(tags: &[String]) -> Viewer {
	if tags.iter().any(|tag| tag == "Manhwa" || tag == "Manhua") {
		Viewer::Webtoon
	} else {
		Viewer::RightToLeft
	}
}

fn parse_content_rating(tags: &[String]) -> ContentRating {
	if tags
		.iter()
		.any(|tag| matches!(tag.as_str(), "Adult" | "Hentai" | "Smut" | "Manhwa 18+"))
	{
		ContentRating::NSFW
	} else if tags.iter().any(|tag| tag == "Ecchi" || tag == "Mature") {
		ContentRating::Suggestive
	} else {
		ContentRating::Safe
	}
}

pub fn parse_chapter_images(script: &str) -> Result<Vec<String>> {
	let json = script
		.split_once("JSON.parse(`")
		.and_then(|(_, rest)| rest.split_once("`)").map(|(json, _)| json))
		.ok_or_else(|| error!("Invalid chapter image data"))?;
	serde_json::from_str(json).map_err(Into::into)
}

pub fn chapter_manga_key(html: &Document) -> Option<String> {
	let json = html
		.select_first(".yoast-schema-graph")
		.and_then(|script| script.data())?;
	let value: serde_json::Value = serde_json::from_str(&json).ok()?;
	let url = value
		.get("@graph")?
		.as_array()?
		.iter()
		.find(|entry| entry.get("@type").and_then(|value| value.as_str()) == Some("WebPage"))?
		.get("url")?
		.as_str()?;
	let path = url.strip_prefix(BASE_URL).unwrap_or(url);
	let mut segments = path.split('/').filter(|segment| !segment.is_empty());
	match (segments.next(), segments.next()) {
		(Some("title-detail"), Some(key)) => Some(key.into()),
		_ => None,
	}
}
