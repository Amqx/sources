#![no_std]
use aidoku::{
	Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, ImageRequestProvider, Listing,
	ListingProvider, Manga, MangaPageResult, Page, PageContent, PageContext, Result, Source,
	alloc::{String, Vec, string::ToString, vec},
	helpers::uri::encode_uri_component,
	imports::{
		html::{Document, Html},
		net::{Request, Response},
		std::send_partial_result,
	},
	prelude::*,
};

mod helpers;
mod livewire;
mod models;
mod parser;
mod reader;
mod settings;

use helpers::*;
use livewire::State;
use models::{ChapterListUpdates, PostFilterUpdates};

const BASE_URL: &str = "https://onisaga.com";
const MAX_CHAPTER_ROUNDS: usize = 40;

struct OniSaga;

impl Source for OniSaga {
	fn new() -> Self {
		settings::apply_rate_limit();
		Self
	}

	fn get_search_manga_list(
		&self,
		query: Option<String>,
		page: i32,
		filters: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		let url = match query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
			Some(query) => format!("{BASE_URL}/search/{}", encode_uri_component(query)),
			None => browse_url(),
		};
		fetch_list(&url, page, updates_from(filters))
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let url = manga_url(&manga.key);
		let response = Request::get(&url)?.send()?;
		let body = response.get_string()?;
		let doc = Html::parse_with_url(&body, &url)?;

		if needs_details {
			let details = parser::parse_details(&doc, manga.key.clone())
				.ok_or(error!("Could not parse the title's details"))?;
			manga.copy_from(details);

			if needs_chapters {
				send_partial_result(&manga);
			}
		}

		if needs_chapters {
			manga.chapters = Some(fetch_chapters(&body, &doc, &url)?);
		}

		Ok(manga)
	}

	fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let url = chapter_url(&chapter.key);
		let body = Request::get(&url)?.string()?;

		let token = reader::token(&body).ok_or(error!("Could not find the reader token"))?;
		let context = reader::context(&token, &url);

		Ok((0..reader::page_count(&body))
			.map(|order| Page {
				content: PageContent::url_context(
					reader::page_url(&chapter.key, order),
					context.clone(),
				),
				..Default::default()
			})
			.collect())
	}
}

impl ListingProvider for OniSaga {
	fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
		let sort = match listing.id.as_str() {
			"popular" => "view",
			"latest" => "created_at",
			_ => bail!("Unknown listing"),
		};
		let updates = PostFilterUpdates {
			platform: settings::default_type(),
			status: settings::default_status(),
			sort: sort.into(),
			..Default::default()
		};
		fetch_list(&browse_url(), page, updates)
	}
}

impl ImageRequestProvider for OniSaga {
	fn get_image_request(&self, url: String, context: Option<PageContext>) -> Result<Request> {
		let Some(context) = context.filter(|_| reader::is_page_url(&url)) else {
			return reader::image_request(url, &format!("{BASE_URL}/"));
		};
		let image = reader::resolve(&url, &context)?;
		let referer = context
			.get(reader::REFERER_CONTEXT_KEY)
			.cloned()
			.unwrap_or_else(|| format!("{BASE_URL}/"));
		reader::image_request(image, &referer)
	}
}

impl DeepLinkHandler for OniSaga {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		let path = url.strip_prefix(BASE_URL).unwrap_or(&url);

		if let Some(key) = manga_slug(path) {
			return Ok(Some(DeepLinkResult::Manga { key }));
		}

		if path.starts_with("/read/") {
			// The reader page is the only place that names the title a chapter
			// belongs to, so it has to be fetched to resolve the link.
			let manga_key = Request::get(&url)?
				.html()?
				.select_first("a[href*=\"/manga/\"]")
				.and_then(|link| link.attr("href"))
				.and_then(|href| manga_slug(&href));
			return Ok(manga_key.map(|manga_key| DeepLinkResult::Chapter {
				manga_key,
				key: to_path(&url),
			}));
		}

		Ok(None)
	}
}

fn browse_url() -> String {
	format!("{BASE_URL}/browse")
}

/// Fetch one page of the browse/search list.
fn fetch_list(url: &str, page: i32, mut updates: PostFilterUpdates) -> Result<MangaPageResult> {
	let excluded = settings::excluded_genres();

	let response = Request::get(url)?.send()?;
	let body = response.get_string()?;
	let doc = Html::parse_with_url(&body, url)?;

	if page <= 1 && updates.is_default() && excluded.is_empty() {
		return Ok(result(&doc, &body));
	}

	for genre in excluded {
		if !updates.exclude_genre.contains(&genre) {
			updates.exclude_genre.push(genre);
		}
	}

	let state = State::extract(&body, &doc, "post-filter")?;
	let response = livewire::request(
		&state.token,
		&state.snapshot,
		url,
		updates,
		"gotoPage",
		vec![page.to_string()],
	)?
	.send()?;

	let (html, _) = livewire::parse(response).ok_or(error!("Empty response from the site"))?;
	let doc = Html::parse_fragment_with_url(&html, format!("{BASE_URL}/"))?;
	Ok(result(&doc, &html))
}

fn result(doc: &Document, html: &str) -> MangaPageResult {
	let entries = parser::parse_manga_list(doc);
	MangaPageResult {
		has_next_page: !entries.is_empty() && livewire::has_next_page(html),
		entries,
	}
}

/// Load every chapter, for every language the user reads.
fn fetch_chapters(body: &str, doc: &Document, url: &str) -> Result<Vec<Chapter>> {
	let Ok(state) = State::extract(body, doc, "manga.chapter-list") else {
		return Ok(Vec::new());
	};

	let mut chains = settings::languages()
		.into_iter()
		.map(|(language, code)| Chain {
			language,
			code,
			snapshot: state.snapshot.clone(),
			chapters: Vec::new(),
			done: false,
		})
		.collect::<Vec<_>>();

	for _ in 0..MAX_CHAPTER_ROUNDS {
		let pending = chains.iter().filter(|chain| !chain.done).count();
		if pending == 0 {
			break;
		}

		let requests = chains
			.iter()
			.filter(|chain| !chain.done)
			.map(|chain| {
				livewire::request(
					&state.token,
					&chain.snapshot,
					url,
					ChapterListUpdates {
						language: chain.code,
					},
					"loadMoreChapters",
					Vec::new(),
				)
			})
			.collect::<Result<Vec<_>>>()?;

		let mut responses = Request::send_all(requests).into_iter();
		for chain in chains.iter_mut().filter(|chain| !chain.done) {
			chain.advance(responses.next().and_then(|response| response.ok()));
		}
	}

	let mut chapters = Vec::new();
	for chain in chains {
		for chapter in chain.chapters {
			if !chapters
				.iter()
				.any(|existing: &Chapter| existing.key == chapter.key)
			{
				chapters.push(chapter);
			}
		}
	}
	parser::sort_chapters(&mut chapters);
	Ok(chapters)
}

/// One language's progress through the chapter list.
struct Chain {
	language: &'static str,
	code: &'static str,
	snapshot: String,
	chapters: Vec<Chapter>,
	done: bool,
}

impl Chain {
	fn advance(&mut self, response: Option<Response>) {
		let Some((html, snapshot)) = response.and_then(livewire::parse) else {
			self.done = true;
			return;
		};
		let Ok(doc) = Html::parse_fragment_with_url(&html, format!("{BASE_URL}/")) else {
			self.done = true;
			return;
		};

		let chapters = parser::parse_chapters(&doc, self.language);
		if chapters.len() <= self.chapters.len() {
			self.done = true;
			return;
		}

		self.chapters = chapters;
		self.snapshot = snapshot;
	}
}

/// Translate Aidoku's filter values into the `post-filter` component's properties.
fn updates_from(filters: Vec<FilterValue>) -> PostFilterUpdates {
	let mut updates = PostFilterUpdates::default();

	for filter in filters {
		match filter {
			FilterValue::Select { id, value } => match id.as_str() {
				"type" => updates.platform = value,
				"status" => updates.status = value,
				"minChapters" => updates.min_chapters = value,
				_ => {}
			},
			FilterValue::Text { id, value } => {
				let value = value.trim();
				if value.is_empty() {
					continue;
				}
				match id.as_str() {
					"group" => updates.group = Some(value.into()),
					"releaseStart" => updates.release_start = Some(value.into()),
					"releaseEnd" => updates.release_end = Some(value.into()),
					_ => {}
				}
			}
			FilterValue::MultiSelect {
				id,
				included,
				excluded,
			} => {
				if id == "genres" {
					updates.genre = included;
					updates.exclude_genre = excluded;
				}
			}
			FilterValue::Sort { index, .. } => {
				updates.sort = match index {
					1 => "view",
					2 => "release_date",
					3 => "like_count",
					4 => "title",
					5 => "vote_average",
					6 => "fan_favorites",
					_ => "created_at",
				}
				.into();
			}
			_ => {}
		}
	}

	updates
}

register_source!(
	OniSaga,
	ListingProvider,
	ImageRequestProvider,
	DeepLinkHandler
);
