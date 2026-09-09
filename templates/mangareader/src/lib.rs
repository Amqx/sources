#![no_std]
use aidoku::{
	Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeLayout, ImageRequestProvider,
	ImageResponse, Listing, ListingProvider, Manga, MangaPageResult, Page, PageContext,
	PageImageProcessor, Result, Source,
	alloc::{String, Vec, borrow::Cow},
	helpers::uri::QueryParameters,
	imports::{canvas::ImageRef, html::Element, net::Request},
	prelude::*,
};

mod helper;
mod imp;
pub mod parser;

pub use helper::slice_count;
pub use imp::Impl;

pub struct Params {
	pub base_url: Cow<'static, str>,
	pub search_path: Cow<'static, str>,
	// path of the page the home layout is read off
	pub home_path: Cow<'static, str>,
	pub search_param: Cow<'static, str>,
	pub page_param: Cow<'static, str>,
	pub page_selector: Cow<'static, str>,
	// css selector for chapter list items (typically contained in #{lang}-chapters or #{lang}-chaps)
	pub get_chapter_selector: fn() -> Cow<'static, str>,
	// the language of a chapter
	pub get_chapter_language: fn(&Element) -> String,
	// some sites only repeat the chapter number in the name, leaving no title to show
	pub has_chapter_titles: bool,
	// path added to base url for page list ajax request
	pub get_page_url_path: fn(&str) -> String,
	pub set_default_filters: fn(&mut QueryParameters) -> (),
	// some sites stack every page of a chapter into one tall image. set to how many times its
	// width a page stands to have those sliced apart
	pub stacked_page_ratio: Option<f32>,
}

impl Default for Params {
	fn default() -> Self {
		Self {
			base_url: "".into(),
			search_path: "/search".into(),
			home_path: "/home".into(),
			search_param: "keyword".into(),
			page_param: "page".into(),
			page_selector: ".container-reader-chapter > div > img".into(),
			get_chapter_selector: || "#en-chapters > li".into(),
			get_chapter_language: |_| "en".into(),
			has_chapter_titles: true,
			get_page_url_path: |chapter_id| format!("//ajax/image/list/{chapter_id}?mode=vertical"),
			set_default_filters: |_| {},
			stacked_page_ratio: None,
		}
	}
}

pub struct MangaReader<T: Impl> {
	inner: T,
	params: Params,
}

impl<T: Impl> Source for MangaReader<T> {
	fn new() -> Self {
		let inner = T::new();
		let params = inner.params();
		Self { inner, params }
	}

	fn get_search_manga_list(
		&self,
		query: Option<String>,
		page: i32,
		filters: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		self.inner
			.get_search_manga_list(&self.params, query, page, filters)
	}

	fn get_manga_update(
		&self,
		manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		self.inner
			.get_manga_update(&self.params, manga, needs_details, needs_chapters)
	}

	fn get_page_list(&self, manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		self.inner.get_page_list(&self.params, manga, chapter)
	}
}

impl<T: Impl> ListingProvider for MangaReader<T> {
	fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
		self.inner.get_manga_list(&self.params, listing, page)
	}
}

impl<T: Impl> Home for MangaReader<T> {
	fn get_home(&self) -> Result<HomeLayout> {
		self.inner.get_home(&self.params)
	}
}

impl<T: Impl> ImageRequestProvider for MangaReader<T> {
	fn get_image_request(&self, url: String, context: Option<PageContext>) -> Result<Request> {
		self.inner.get_image_request(&self.params, url, context)
	}
}

impl<T: Impl> PageImageProcessor for MangaReader<T> {
	fn process_page_image(
		&self,
		response: ImageResponse,
		context: Option<PageContext>,
	) -> Result<ImageRef> {
		self.inner
			.process_page_image(&self.params, response, context)
	}
}

impl<T: Impl> DeepLinkHandler for MangaReader<T> {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		self.inner.handle_deep_link(&self.params, url)
	}
}
