#![no_std]
use aidoku::{Source, alloc::borrow::Cow, prelude::*};
use mangareader::{Impl, MangaReader, Params};

const BASE_URL: &str = "https://mangamura.me";
// some chapters ship as one tall jpeg; across 33 of them a page ran 1.39 to 1.44 times its width
const PAGE_ASPECT: f32 = 1.42;

struct MangaMura;

impl Impl for MangaMura {
	fn new() -> Self {
		Self
	}

	fn params(&self) -> Params {
		Params {
			base_url: BASE_URL.into(),
			search_path: "".into(),
			// the site has no /home; the root carries the same layout
			home_path: "/".into(),
			search_param: "q".into(),
			page_param: "p".into(),
			get_chapter_selector: || "#ja-chaps > li".into(),
			get_chapter_language: |_| "ja".into(),
			// chapter names only ever repeat the number
			has_chapter_titles: false,
			get_page_url_path: |chapter_id| format!("/json/chapter?id={chapter_id}&mode=vertical"),
			set_default_filters: |query_params| {
				query_params.set("type", Some("all"));
				query_params.set("status", Some("all"));
				query_params.set("language", Some("all"));
				query_params.set("sort", Some("default"));
			},
			stacked_page_ratio: Some(PAGE_ASPECT),
			..Default::default()
		}
	}

	fn get_sort_id(&self, index: i32) -> Cow<'static, str> {
		match index {
			0 => "default",
			1 => "latest-update",
			2 => "most-viewed",
			3 => "title-az",
			4 => "title-za",
			_ => "default",
		}
		.into()
	}
}

register_source!(
	MangaReader<MangaMura>,
	ListingProvider,
	Home,
	ImageRequestProvider,
	PageImageProcessor,
	DeepLinkHandler
);

#[cfg(test)]
mod test;
