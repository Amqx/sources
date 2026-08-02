#![no_std]
use aidoku::{
	Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeComponent, HomeComponentValue,
	HomeLayout, HomePartialResult, ImageRequestProvider, Link, Listing, ListingProvider, Manga,
	MangaPageResult, Page, PageContent, PageContext, Result, Source,
	alloc::{String, Vec, format},
	imports::{
		net::{Request, TimeUnit, set_rate_limit},
		std::send_partial_result,
	},
	prelude::*,
};

mod helpers;
mod models;
use helpers::*;
use models::*;

struct Atsumaru;

impl Source for Atsumaru {
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
		let url = search_url(query.as_deref(), page, &filters);
		let response: SearchResponse = api_json(&url)?;

		Ok(MangaPageResult {
			entries: response
				.hits
				.into_iter()
				.map(|hit| hit.document.into())
				.collect(),
			has_next_page: page * PER_PAGE < response.found,
		})
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let mut manga_page =
			api_json::<MangaPageResponse>(&format!("{BASE_URL}/api/manga/page?id={}", manga.key))?
				.manga_page;

		// The manga page carries the scanlator names that the chapter list only
		// references by id, along with the first chapters of the series. Both
		// are taken before the rest of the page is consumed for details.
		let scanlators = manga_page.scanlators.take().unwrap_or_default();
		let complete_chapters = (needs_chapters && manga_page.has_more_chapters == Some(false))
			.then(|| manga_page.chapters.take())
			.flatten();

		if needs_details {
			manga_page.fill_details(&mut manga);
			if needs_chapters && complete_chapters.is_none() {
				send_partial_result(&manga);
			}
		}

		if needs_chapters {
			let chapters = match complete_chapters {
				Some(chapters) => chapters,
				None => {
					api_json::<AllChaptersResponse>(&format!(
						"{BASE_URL}/api/manga/allChapters?mangaId={}",
						manga.key
					))?
					.chapters
				}
			};

			let mut chapters: Vec<Chapter> = chapters
				.into_iter()
				.map(|chapter| chapter.into_chapter(&manga.key, &scanlators))
				.collect();

			// Series with several scanlators list a chapter once per group, so
			// every duplicate is kept, grouped under its chapter number.
			chapters.sort_by(|a, b| {
				b.chapter_number
					.partial_cmp(&a.chapter_number)
					.unwrap_or(core::cmp::Ordering::Equal)
					.then_with(|| a.scanlators.cmp(&b.scanlators))
					.then_with(|| b.date_uploaded.cmp(&a.date_uploaded))
			});

			manga.chapters = Some(chapters);
		}

		Ok(manga)
	}

	fn get_page_list(&self, manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let url = format!(
			"{BASE_URL}/api/read/chapter?mangaId={}&chapterId={}",
			manga.key, chapter.key
		);
		let response: ReadChapterResponse = api_json(&url)?;

		Ok(response
			.read_chapter
			.pages
			.into_iter()
			.map(|page| Page {
				content: PageContent::url(image_url(&page.image)),
				..Default::default()
			})
			.collect())
	}
}

impl ListingProvider for Atsumaru {
	fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
		let response: ListingResponse = api_json(&listing_url(&listing.id, page))?;
		let entries: Vec<Manga> = response.items.into_iter().map(Into::into).collect();

		Ok(MangaPageResult {
			has_next_page: !entries.is_empty(),
			entries,
		})
	}
}

impl Home for Atsumaru {
	fn get_home(&self) -> Result<HomeLayout> {
		const LISTINGS: [(&str, &str); 4] = [
			("trending", "Trending"),
			("popular", "Popular"),
			("recentlyUpdated", "Recently Updated"),
			("recentlyAdded", "Recently Added"),
		];

		send_partial_result(&HomePartialResult::Layout(HomeLayout {
			components: LISTINGS
				.iter()
				.map(|(_, name)| HomeComponent {
					title: Some((*name).into()),
					subtitle: None,
					value: HomeComponentValue::empty_scroller(),
				})
				.collect(),
		}));

		let requests = LISTINGS
			.iter()
			.map(|(id, _)| api_get(&listing_url(id, 1)))
			.collect::<Result<Vec<Request>>>()?;

		for (response, (id, name)) in Request::send_all(requests).into_iter().zip(LISTINGS) {
			let entries = response
				.map_err(Into::into)
				.and_then(response_json::<ListingResponse>)
				.map(|response| {
					response
						.items
						.into_iter()
						.map(|item| Manga::from(item).into())
						.collect::<Vec<Link>>()
				})
				.unwrap_or_default();

			send_partial_result(&HomePartialResult::Component(HomeComponent {
				title: Some(name.into()),
				subtitle: None,
				value: HomeComponentValue::Scroller {
					entries,
					listing: Some(Listing {
						id: id.into(),
						name: name.into(),
						..Default::default()
					}),
				},
			}));
		}

		Ok(HomeLayout::default())
	}
}

impl ImageRequestProvider for Atsumaru {
	fn get_image_request(&self, url: String, _context: Option<PageContext>) -> Result<Request> {
		Ok(Request::get(url)?
			.header("Accept", "image/avif,image/webp,*/*")
			.header("Referer", &format!("{BASE_URL}/")))
	}
}

impl DeepLinkHandler for Atsumaru {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		let Some(path) = url.strip_prefix(BASE_URL) else {
			return Ok(None);
		};
		let path = path.split(['?', '#']).next().unwrap_or(path);
		let mut segments = path.split('/').filter(|segment| !segment.is_empty());

		Ok(match (segments.next(), segments.next(), segments.next()) {
			// https://atsu.moe/manga/<manga id>
			(Some("manga"), Some(key), None) => Some(DeepLinkResult::Manga { key: key.into() }),
			// https://atsu.moe/read/<manga id>/<chapter id>
			(Some("read"), Some(manga_key), Some(key)) => Some(DeepLinkResult::Chapter {
				manga_key: manga_key.into(),
				key: key.into(),
			}),
			_ => None,
		})
	}
}

register_source!(
	Atsumaru,
	ListingProvider,
	Home,
	ImageRequestProvider,
	DeepLinkHandler
);

#[cfg(test)]
mod tests {
	use super::*;
	use aidoku::{ContentRating, MangaStatus, Viewer, alloc::vec};
	use aidoku_test::aidoku_test;

	/// Sakamoto Days, a long running series with a single scanlator.
	const TEST_MANGA_KEY: &str = "v8Kbg";

	#[aidoku_test]
	fn search_url_includes_every_filter() {
		let url = search_url(
			Some("sakamoto"),
			2,
			&[
				FilterValue::Sort {
					id: "sort".into(),
					index: 1,
					ascending: false,
				},
				FilterValue::MultiSelect {
					id: "genre".into(),
					included: vec!["39".into(), "6".into()],
					excluded: vec!["10".into()],
				},
				FilterValue::MultiSelect {
					id: "tag".into(),
					included: vec!["8".into()],
					excluded: vec!["16".into()],
				},
				FilterValue::MultiSelect {
					id: "type".into(),
					included: vec!["Manga".into()],
					excluded: vec![],
				},
				FilterValue::MultiSelect {
					id: "status".into(),
					included: vec!["Ongoing".into()],
					excluded: vec![],
				},
				FilterValue::Range {
					id: "year".into(),
					from: Some(2010.0),
					to: Some(2024.0),
				},
				FilterValue::Range {
					id: "chapters".into(),
					from: Some(50.0),
					to: None,
				},
				FilterValue::Check {
					id: "official".into(),
					value: 1,
				},
			],
		);

		let decoded = aidoku::helpers::uri::decode_uri(&url);
		assert!(decoded.contains("q=sakamoto"), "{decoded}");
		assert!(
			decoded.contains("genreIds:=`39` && genreIds:=`6`"),
			"{decoded}"
		);
		assert!(decoded.contains("genreIds:!=[`10`]"), "{decoded}");
		assert!(decoded.contains("tagIds:=`8`"), "{decoded}");
		assert!(decoded.contains("tagIds:!=[`16`]"), "{decoded}");
		assert!(decoded.contains("type:=[`Manga`]"), "{decoded}");
		assert!(decoded.contains("status:=[`Ongoing`]"), "{decoded}");
		assert!(decoded.contains("releaseYear:>=2010"), "{decoded}");
		assert!(decoded.contains("releaseYear:<=2024"), "{decoded}");
		assert!(decoded.contains("chapterCount:>=50"), "{decoded}");
		assert!(!decoded.contains("chapterCount:<="), "{decoded}");
		assert!(decoded.contains("isAdult:=false"), "{decoded}");
		assert!(decoded.contains("officialTranslation:=true"), "{decoded}");
		assert!(decoded.contains("sort_by=views:desc"), "{decoded}");
		assert!(decoded.contains("page=2"), "{decoded}");
	}

	#[aidoku_test]
	fn search_url_sorts_by_views_when_browsing_by_relevance() {
		let browse = aidoku::helpers::uri::decode_uri(search_url(None, 1, &[]));
		assert!(browse.contains("q=*"), "{browse}");
		assert!(browse.contains("sort_by=views:desc"), "{browse}");
		assert!(!browse.contains("query_by"), "{browse}");

		// with a search term, the search engine's own ranking is used instead
		let query = aidoku::helpers::uri::decode_uri(search_url(Some("blue lock"), 1, &[]));
		assert!(!query.contains("sort_by"), "{query}");
		assert!(
			query.contains("query_by=title,englishTitle,otherNames,authors"),
			"{query}"
		);
	}

	#[aidoku_test]
	fn image_urls_point_at_the_cdn_over_https() {
		assert_eq!(
			image_url("posters/abc.jpg"),
			"https://cdn.atsu.moe/static/posters/abc.jpg"
		);
		assert_eq!(
			image_url("/static/pages/a/b/0.webp"),
			"https://cdn.atsu.moe/static/pages/a/b/0.webp"
		);
		assert_eq!(
			image_url("//cdn.example/a.jpg"),
			"https://cdn.example/a.jpg"
		);
		assert_eq!(
			image_url("http://cdn.example/a.jpg"),
			"https://cdn.example/a.jpg"
		);
		assert_eq!(
			image_url("https//cdn.example/a.jpg"),
			"https://cdn.example/a.jpg"
		);
	}

	#[aidoku_test]
	fn chapter_titles_that_only_repeat_the_number_are_dropped() {
		assert_eq!(
			clean_chapter_title(Some("Chapter 12".into()), Some(12.0)),
			None
		);
		assert_eq!(clean_chapter_title(Some("12".into()), Some(12.0)), None);
		assert_eq!(clean_chapter_title(Some("  ".into()), Some(12.0)), None);
		assert_eq!(
			clean_chapter_title(Some("Days 270".into()), Some(270.0)),
			Some("Days 270".into())
		);
		assert_eq!(
			clean_chapter_title(Some("Chapter 12.5".into()), Some(12.5)),
			None
		);
	}

	#[aidoku_test]
	fn years_are_read_from_millisecond_timestamps() {
		assert_eq!(year_from_millis(1577836800000), 2020);
		assert_eq!(year_from_millis(0), 1970);
		assert_eq!(year_from_millis(-86_400_000), 1969);
	}

	#[aidoku_test]
	fn deep_links_resolve_to_manga_and_chapters() {
		let source = Atsumaru;
		assert_eq!(
			source
				.handle_deep_link("https://atsu.moe/manga/v8Kbg".into())
				.unwrap(),
			Some(DeepLinkResult::Manga {
				key: "v8Kbg".into()
			})
		);
		assert_eq!(
			source
				.handle_deep_link("https://atsu.moe/read/v8Kbg/P7297ifE".into())
				.unwrap(),
			Some(DeepLinkResult::Chapter {
				manga_key: "v8Kbg".into(),
				key: "P7297ifE".into()
			})
		);
		assert_eq!(
			source.handle_deep_link("https://atsu.moe/".into()).unwrap(),
			None
		);
		assert_eq!(
			source
				.handle_deep_link("https://example.com/manga/v8Kbg".into())
				.unwrap(),
			None
		);
	}

	#[aidoku_test]
	fn search_returns_results() {
		let source = Atsumaru;
		let result = source
			.get_search_manga_list(Some("sakamoto days".into()), 1, Vec::new())
			.expect("search request failed");

		assert!(!result.entries.is_empty());
		let manga = result
			.entries
			.iter()
			.find(|manga| manga.key == TEST_MANGA_KEY)
			.expect("Sakamoto Days should be in the results");
		assert_eq!(manga.title, "Sakamoto Days");
		assert!(manga.cover.as_ref().unwrap().starts_with("https://"));
	}

	/// Every sort field has to exist in the search collection's schema, or the
	/// api answers with a 404 that used to read as an empty result page.
	#[aidoku_test]
	fn every_sort_option_returns_results() {
		let source = Atsumaru;
		for index in 0..7 {
			let result = source
				.get_search_manga_list(
					None,
					1,
					vec![FilterValue::Sort {
						id: "sort".into(),
						index,
						ascending: false,
					}],
				)
				.unwrap_or_else(|error| panic!("sort option {index} failed: {error:?}"));
			assert!(!result.entries.is_empty(), "sort option {index} was empty");
		}
	}

	#[aidoku_test]
	fn listings_return_results() {
		let source = Atsumaru;
		for id in ["trending", "popular", "recentlyUpdated", "recentlyAdded"] {
			let result = source
				.get_manga_list(
					Listing {
						id: id.into(),
						name: id.into(),
						..Default::default()
					},
					1,
				)
				.expect("listing request failed");
			assert!(!result.entries.is_empty(), "{id} returned nothing");
			assert!(result.has_next_page);
		}
	}

	#[aidoku_test]
	fn home_components_are_filled() {
		let source = Atsumaru;
		source.get_home().expect("home request failed");
	}

	#[aidoku_test]
	fn manga_details_and_chapters_are_parsed() {
		let source = Atsumaru;
		let manga = source
			.get_manga_update(
				Manga {
					key: TEST_MANGA_KEY.into(),
					..Default::default()
				},
				true,
				true,
			)
			.expect("manga page request failed");

		assert_eq!(manga.title, "Sakamoto Days");
		assert_eq!(manga.status, MangaStatus::Ongoing);
		assert_eq!(manga.viewer, Viewer::RightToLeft);
		assert_eq!(manga.content_rating, ContentRating::Safe);
		assert_eq!(manga.url.as_deref(), Some("https://atsu.moe/manga/v8Kbg"));
		assert!(manga.cover.is_some());
		assert!(manga.authors.is_some());
		assert!(manga.artists.is_some());
		assert!(
			manga
				.tags
				.as_ref()
				.unwrap()
				.contains(&String::from("Action"))
		);

		let description = manga.description.expect("missing description");
		assert!(description.contains("Rating: "), "{description}");
		assert!(description.contains("Alternative Names:"), "{description}");

		let chapters = manga.chapters.expect("missing chapters");
		assert!(chapters.len() > 100);
		// newest first
		assert!(chapters[0].chapter_number > chapters[1].chapter_number);
		let last = chapters.last().unwrap();
		assert_eq!(last.chapter_number, Some(1.0));
		assert!(last.date_uploaded.is_some());
		assert!(last.scanlators.is_some());
		assert_eq!(
			last.url.as_deref(),
			Some("https://atsu.moe/read/v8Kbg/P7297ifE")
		);
	}

	#[aidoku_test]
	fn page_list_is_parsed() {
		let source = Atsumaru;
		let pages = source
			.get_page_list(
				Manga {
					key: TEST_MANGA_KEY.into(),
					..Default::default()
				},
				Chapter {
					key: "P7297ifE".into(),
					..Default::default()
				},
			)
			.expect("page list request failed");

		assert!(!pages.is_empty());
		let PageContent::Url(url, _) = &pages[0].content else {
			panic!("expected a page url");
		};
		assert!(
			url.starts_with("https://cdn.atsu.moe/static/pages/"),
			"{url}"
		);
	}
}
