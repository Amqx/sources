use super::*;
use aidoku::{
	Chapter, Home, HomeComponentValue, Manga, PageContent,
	alloc::{String, vec::Vec},
};
use aidoku_test::aidoku_test;
use mangareader::slice_count;

const STACKED_MANGA_KEY: &str = "/manga/小林さんちのメイドラゴン-イルルは恋とかわかりません！-raw/";
const PAGED_MANGA_KEY: &str = "/manga/せっかく農家に転生したので勇者は目指しません-raw/";

fn source() -> MangaReader<MangaMura> {
	Source::new()
}

fn fetch_chapter(key: &str, number: f32) -> (Manga, Chapter) {
	let manga = Manga {
		key: String::from(key),
		..Default::default()
	};
	let mut manga = source()
		.get_manga_update(manga, false, true)
		.expect("get_manga_update failed");
	let chapter = manga
		.chapters
		.take()
		.expect("no chapters returned")
		.into_iter()
		.find(|chapter| chapter.chapter_number == Some(number))
		.unwrap_or_else(|| panic!("chapter {number} is gone"));
	(manga, chapter)
}

// entries are linked with absolute urls, so a stale BASE_URL leaves every key a full url and
// breaks details and chapter lists
#[aidoku_test]
fn search_returns_path_keys() {
	let result = source()
		.get_search_manga_list(Some(String::from("ワンピース")), 1, Vec::new())
		.expect("search failed");
	assert!(!result.entries.is_empty(), "expected at least one result");
	for manga in &result.entries {
		assert!(
			manga.key.starts_with('/'),
			"expected a path key, got {}",
			manga.key
		);
	}
}

#[aidoku_test]
fn manga_details_have_chapters() {
	let source = source();
	let manga = source
		.get_search_manga_list(Some(String::from("ワンピース")), 1, Vec::new())
		.expect("search failed")
		.entries
		.into_iter()
		.next()
		.expect("expected at least one result");
	let manga = source
		.get_manga_update(manga, true, true)
		.expect("get_manga_update failed");
	let chapters = manga.chapters.expect("no chapters returned");
	assert!(chapters.len() > 100, "got {} chapters", chapters.len());
}

// chapter 1 is served as one 800x18208 jpg stacking all 16 pages
#[aidoku_test]
fn stacked_chapter_is_split() {
	let (manga, chapter) = fetch_chapter(STACKED_MANGA_KEY, 1.0);
	let pages = source().get_page_list(manga, chapter).expect("pages");
	assert!(pages.len() > 8, "got {} pages", pages.len());

	let mut first_url = None;
	for (index, page) in pages.iter().enumerate() {
		let PageContent::Url(url, Some(context)) = &page.content else {
			panic!("page {index} carries no slice");
		};
		let first_url = first_url.get_or_insert_with(|| url.clone());
		assert_eq!(url, first_url, "the slices come off one image");
		assert_eq!(
			context.get("slice").map(|slice| slice.parse::<usize>()),
			Some(Ok(index)),
			"slice {index} is out of order"
		);
		assert_eq!(
			context.get("slices").map(|slices| slices.parse::<usize>()),
			Some(Ok(pages.len())),
			"the slice count does not match the pages handed over"
		);
	}
}

// a chapter holding a page per image is handed over untouched
#[aidoku_test]
fn paged_chapter_is_left_alone() {
	let (manga, chapter) = fetch_chapter(PAGED_MANGA_KEY, 1.0);
	let pages = source().get_page_list(manga, chapter).expect("pages");
	assert!(pages.len() > 4, "got {} pages", pages.len());
	for (index, page) in pages.iter().enumerate() {
		let PageContent::Url(_, context) = &page.content else {
			panic!("page {index} is not a url");
		};
		assert!(context.is_none(), "page {index} was sliced");
	}
}

// sizes read off the site, and headers too broken to measure
#[aidoku_test]
fn slice_count_matches_measured_images() {
	assert_eq!(slice_count_at(800, 18208), 16);
	assert_eq!(slice_count_at(800, 27312), 24);
	assert_eq!(slice_count_at(1426, 53248), 26);
	assert_eq!(slice_count_at(1115, 56000), 35);
	// 38912 / 19 lands on 2048; the plain ratio rounds 19.53 up to 20 and cuts every page
	assert_eq!(slice_count_at(1403, 38912), 19);
	assert_eq!(slice_count_at(1125, 64000), 40);
	assert_eq!(slice_count_at(650, 924), 1);
	assert_eq!(slice_count_at(0, 49152), 1);
	assert_eq!(slice_count_at(0, 0), 1);
	assert_eq!(slice_count_at(100, 49152), 1);
}

fn slice_count_at(width: u32, height: u32) -> u32 {
	slice_count(width, height, PAGE_ASPECT)
}

#[aidoku_test]
fn manga_details_fill_title() {
	let manga = Manga {
		key: String::from(STACKED_MANGA_KEY),
		..Default::default()
	};
	let manga = source()
		.get_manga_update(manga, true, false)
		.expect("get_manga_update failed");
	assert_eq!(
		manga.title,
		"小林さんちのメイドラゴン イルルは恋とかわかりません！"
	);
}

#[aidoku_test]
fn home_has_entries() {
	let home = source().get_home().expect("get_home failed");
	assert!(!home.components.is_empty(), "no home components");
	let filled = home
		.components
		.iter()
		.filter(|component| match &component.value {
			HomeComponentValue::BigScroller { entries, .. } => !entries.is_empty(),
			HomeComponentValue::Scroller { entries, .. } => !entries.is_empty(),
			HomeComponentValue::MangaList { entries, .. } => !entries.is_empty(),
			HomeComponentValue::MangaChapterList { entries, .. } => !entries.is_empty(),
			_ => false,
		})
		.count();
	assert!(filled > 2, "only {filled} components carry entries");
}

// every chapter name repeats the number, bare or with a volume suffix or the seo heading
#[aidoku_test]
fn chapters_carry_no_title() {
	let manga = Manga {
		key: String::from("/manga/約束のネバーランド-raw/"),
		..Default::default()
	};
	let manga = source()
		.get_manga_update(manga, false, true)
		.expect("get_manga_update failed");
	let chapters = manga.chapters.expect("no chapters returned");
	assert!(
		chapters.iter().any(|c| c.chapter_number == Some(181.6)),
		"the decimal chapter is gone"
	);
	for chapter in &chapters {
		assert!(
			chapter.title.is_none(),
			"chapter {:?} kept the title {:?}",
			chapter.chapter_number,
			chapter.title
		);
	}
}
