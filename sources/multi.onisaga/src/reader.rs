use aidoku::{PageContext, Result, alloc::String, imports::net::Request, prelude::*};

use crate::{BASE_URL, models::PageApiResponse};

pub const TOKEN_CONTEXT_KEY: &str = "token";
pub const REFERER_CONTEXT_KEY: &str = "referer";

const API_PATH: &str = "/api/chapter/";

/// Pull `readerToken: "…"` out of the reader page's inline script.
pub fn token(body: &str) -> Option<String> {
	let after = body.split_once("readerToken")?.1;
	let after = after
		.trim_start()
		// the closing quote, when the token is a JSON key rather than a JS one
		.trim_start_matches(['"', '\''])
		.trim_start()
		.strip_prefix(':')?
		.trim_start();
	let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
	let value = after[1..].split(quote).next()?;
	(!value.is_empty()).then(|| value.into())
}

/// Count the `order: N` entries the reader page lists, one per page.
pub fn page_count(body: &str) -> usize {
	let mut count = 0;
	let mut from = 0;
	while let Some(offset) = body[from..].find("order") {
		let idx = from + offset;
		from = idx + "order".len();

		// `border: 1px` also ends in "order:", so require a word boundary.
		if body[..idx]
			.chars()
			.next_back()
			.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
		{
			continue;
		}

		let rest = body[from..].trim_start_matches(['"', '\'']).trim_start();
		let Some(rest) = rest.strip_prefix(':') else {
			continue;
		};
		if rest.trim_start().starts_with(|c: char| c.is_ascii_digit()) {
			count += 1;
		}
	}
	count
}

/// The url that [`resolve`] turns into an image url.
pub fn page_url(chapter_key: &str, order: usize) -> String {
	let id = chapter_key
		.trim_end_matches('/')
		.rsplit('/')
		.next()
		.unwrap_or_default();
	format!("{BASE_URL}{API_PATH}{id}/page/{order}")
}

pub fn is_page_url(url: &str) -> bool {
	url.starts_with(BASE_URL) && url.contains(API_PATH)
}

/// Exchange a page api url for the signed image url behind it.
pub fn resolve(url: &str, context: &PageContext) -> Result<String> {
	let referer = context
		.get(REFERER_CONTEXT_KEY)
		.ok_or(error!("Page is missing its chapter url"))?;
	let mut token = context.get(TOKEN_CONTEXT_KEY).cloned().unwrap_or_default();

	for attempt in 0..2 {
		let response = Request::get(url)?
			.header("Accept", "application/json")
			.header("X-Reader-Token", &token)
			.header("Sec-Fetch-Mode", "cors")
			.header("Sec-Fetch-Site", "same-origin")
			.header("Referer", referer)
			.send()?;

		let status = response.status_code();
		if status == 429 {
			bail!("The site is rate limiting us; try again in a few minutes");
		}

		let page = response
			.get_json_owned::<PageApiResponse>()
			.unwrap_or_default();
		if let Some(image) = page.url {
			return Ok(image);
		}
		if attempt > 0 {
			bail!(
				"{}",
				page.message
					.unwrap_or_else(|| format!("Page request failed (HTTP {status})"))
			);
		}

		token = self::token(&Request::get(referer)?.string()?)
			.ok_or(error!("Could not refresh the reader token"))?;
	}

	bail!("Could not load the page image")
}

/// The context Aidoku hands back to us for each page.
pub fn context(token: &str, chapter_url: &str) -> PageContext {
	PageContext::from_iter([
		(String::from(TOKEN_CONTEXT_KEY), String::from(token)),
		(String::from(REFERER_CONTEXT_KEY), String::from(chapter_url)),
	])
}

/// The site's image host checks the referer.
pub fn image_request(url: String, referer: &str) -> Result<Request> {
	Ok(Request::get(url)?.header("Referer", referer))
}

#[cfg(test)]
mod tests {
	use super::*;
	use aidoku_test::aidoku_test;

	#[aidoku_test]
	fn finds_the_reader_token() {
		assert_eq!(
			token(r#"<script>window.reader = { readerToken: "abc.def", order: 0 }</script>"#)
				.as_deref(),
			Some("abc.def")
		);
		assert_eq!(token(r#"{"readerToken":'xyz'}"#).as_deref(), Some("xyz"));
		assert_eq!(token("<script>var x = 1</script>"), None);
	}

	#[aidoku_test]
	fn counts_pages() {
		assert_eq!(page_count(r#"[{"order":0},{"order":1},{"order":2}]"#), 3);
		assert_eq!(page_count("[{ order: 0 }, { order: 1 }]"), 2);
		// `border: 1px` ends in "order:" too, and must not be counted.
		assert_eq!(page_count(r#"<div style="border: 1px solid"></div>"#), 0);
		assert_eq!(page_count(r#"{"order":"asc"}"#), 0);
	}

	#[aidoku_test]
	fn builds_page_urls() {
		assert_eq!(
			page_url("/read/one-piece/abc123", 4),
			"https://onisaga.com/api/chapter/abc123/page/4"
		);
		assert!(is_page_url("https://onisaga.com/api/chapter/abc123/page/4"));
		assert!(!is_page_url("https://onisaga.com/storage/cover.jpg"));
	}
}
