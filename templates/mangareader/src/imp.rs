use super::{
	Params,
	helper::{ElementImageAttr, stacked_page_count},
	parser,
};
use aidoku::{
	Chapter, DeepLinkResult, FilterValue, HomeComponent, HomeComponentValue, HomeLayout,
	ImageResponse, Listing, Manga, MangaPageResult, MangaWithChapter, Page, PageContent,
	PageContext, Result,
	alloc::{String, Vec, borrow::Cow, string::ToString},
	canvas::Rect,
	helpers::uri::{QueryParameters, encode_uri_component},
	imports::{
		canvas::{Canvas, ImageRef},
		html::{Element, Html},
		net::Request,
		std::send_partial_result,
	},
	prelude::*,
};

// a stacked chapter holds a couple of images at most; past this, skip the measuring requests
const STACKED_IMAGE_LIMIT: usize = 4;

pub trait Impl {
	fn new() -> Self;

	fn params(&self) -> Params;

	fn get_sort_id(&self, index: i32) -> Cow<'static, str> {
		match index {
			0 => "default",
			1 => "latest-updated",
			2 => "score",
			3 => "name-az",
			4 => "release-date",
			5 => "most-viewed",
			_ => "default",
		}
		.into()
	}

	fn get_search_manga_list(
		&self,
		params: &Params,
		query: Option<String>,
		page: i32,
		filters: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		let url = if let Some(query) = query {
			format!(
				"{}{}?{}={}&{}={page}",
				params.base_url,
				params.search_path,
				params.search_param,
				encode_uri_component(query),
				params.page_param
			)
		} else {
			let mut qs = QueryParameters::new();
			(params.set_default_filters)(&mut qs);
			for filter in filters {
				match filter {
					FilterValue::Sort { index, .. } => {
						qs.set("sort", Some(self.get_sort_id(index).as_ref()));
					}
					FilterValue::Select { id, value } => {
						qs.set(&id, Some(&value));
					}
					// genres
					FilterValue::MultiSelect { included, .. } => {
						qs.set("genres", Some(&included.join(",")));
					}
					_ => {}
				}
			}
			format!(
				"{}/filter?{}={page}{}{qs}",
				params.base_url,
				params.page_param,
				if !qs.is_empty() { "&" } else { "" }
			)
		};
		let html = Request::get(&url)?.html()?;

		let entries = parser::parse_response(
			&html,
			params.base_url.as_ref(),
			".manga_list-sbs .manga-poster",
		);

		let has_next_page = html
			.select_first("ul.pagination > li.active + li")
			.is_some();

		Ok(MangaPageResult {
			entries,
			has_next_page,
		})
	}

	fn get_manga_update(
		&self,
		params: &Params,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let url = format!("{}{}", params.base_url, manga.key);
		let html = Request::get(&url)?.html()?;

		if needs_details {
			manga.url = Some(url);
			parser::parse_manga_details(&mut manga, &html)?;
			send_partial_result(&manga);
		}

		if needs_chapters {
			manga.chapters = parser::parse_manga_chapters(&html, params)
		}

		Ok(manga)
	}

	fn get_page_list(&self, params: &Params, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let hash_pos = chapter.key.rfind('#');
		let id: Option<String> = hash_pos
			.map(|pos| (&chapter.key[pos + 1..]).into())
			.or_else(|| {
				// get chapter id from chapter page html
				Request::get(format!("{}{}", params.base_url, chapter.key))
					.and_then(|req| req.html())
					.ok()
					.and_then(|html| html.select_first("div[data-reading-id]"))
					.and_then(|el| el.attr("data-reading-id"))
			});
		let Some(id) = id else {
			bail!("Unable to retrieve chapter id");
		};

		let chapter_key_without_id = hash_pos
			.map(|pos| (&chapter.key[..pos]).into())
			.unwrap_or(chapter.key);

		let url = format!("{}{}", params.base_url, (params.get_page_url_path)(&id));
		let json = Request::get(url)?
			.header("Accept", "application/json, text/javascript, */*; q=0.01")
			.header(
				"Referer",
				&format!("{}{}", params.base_url, chapter_key_without_id),
			)
			.header("X-Requested-With", "XMLHttpRequest")
			.json_owned::<serde_json::Value>()?;
		let html_text = json["html"].as_str().unwrap_or_default();
		let html = Html::parse_fragment(html_text)?;

		let images: Vec<String> = html
			.select(&params.page_selector)
			.map(|els| {
				els.filter_map(|el| {
					let url = el
						.img_attr()
						.or_else(|| el.select_first("img").and_then(|img| img.img_attr()))?;
					Some(String::from(url.trim()))
				})
				.collect()
			})
			.unwrap_or_default();

		let measure = params.stacked_page_ratio.is_some() && images.len() <= STACKED_IMAGE_LIMIT;
		let mut pages = Vec::with_capacity(images.len());
		for url in images {
			let slices = match params.stacked_page_ratio {
				Some(ratio) if measure => stacked_page_count(&url, ratio),
				_ => 1,
			};

			if slices < 2 {
				pages.push(Page {
					content: PageContent::url(url),
					..Default::default()
				});
				continue;
			}

			for slice in 0..slices {
				let mut context = PageContext::default();
				context.insert("slice".into(), slice.to_string());
				context.insert("slices".into(), slices.to_string());
				pages.push(Page {
					content: PageContent::url_context(url.clone(), context),
					..Default::default()
				});
			}
		}

		Ok(pages)
	}

	fn get_manga_list(
		&self,
		params: &Params,
		listing: Listing,
		page: i32,
	) -> Result<MangaPageResult> {
		let url = format!(
			"{}/{}?{}={page}",
			params.base_url, listing.id, params.page_param
		);
		let html = Request::get(url)?.html()?;
		let entries = parser::parse_manga_list(&html, &params.base_url);

		Ok(MangaPageResult {
			entries,
			has_next_page: html.select_first("a.page-link[title=\"Next\"]").is_some(),
		})
	}

	fn get_home(&self, params: &Params) -> Result<HomeLayout> {
		let html = Request::get(format!("{}{}", params.base_url, params.home_path))?.html()?;

		let mut components = Vec::new();

		// header
		if let Some(slider_elements) =
			html.select("#slider .deslide-item:not(.swiper-slide-duplicate)")
		{
			components.push(HomeComponent {
				value: HomeComponentValue::BigScroller {
					entries: slider_elements
						.filter_map(|el| {
							let link = el.select_first(".desi-head-title a")?;
							let link_href = link.attr("href")?;
							Some(Manga {
								key: link_href
									.strip_prefix(params.base_url.as_ref())
									.map(|s| s.into())
									.unwrap_or(link_href),
								title: link.attr("title")?,
								cover: el
									.select_first(".deslide-poster img")
									.and_then(|e| e.img_attr()),
								description: el
									.select_first(".sc-detail > .scd-item")
									.and_then(|e| e.text()),
								tags: el
									.select(".sc-detail > .scd-genres > span")
									.map(|els| els.filter_map(|e| e.text()).collect()),
								..Default::default()
							})
						})
						.collect(),
					auto_scroll_interval: Some(5.0),
				},
				..Default::default()
			});
		}

		fn parse_swiper(params: &Params, section: Element) -> HomeComponent {
			HomeComponent {
				title: section.select(".cat-heading").and_then(|e| e.text()),
				value: HomeComponentValue::Scroller {
					entries: section
						.select(".swiper-slide")
						.map(|els| {
							els.filter_map(|e| {
								let link_href = e.select_first(".manga-poster a")?.attr("href")?;
								Some(
									Manga {
										key: link_href
											.strip_prefix(params.base_url.as_ref())
											.map(|s| s.into())
											.unwrap_or(link_href),
										title: e
											.select_first(".anime-name, .manga-name")?
											.text()?,
										cover: e
											.select_first(".manga-poster img")
											.and_then(|img| img.img_attr()),
										..Default::default()
									}
									.into(),
								)
							})
							.collect()
						})
						.unwrap_or_default(),
					listing: None,
				},
				..Default::default()
			}
		}

		// trending
		if let Some(section) = html.select_first("#manga-trending") {
			components.push(parse_swiper(params, section));
		}

		// recommended
		if let Some(section) = html.select_first("#manga-featured") {
			components.push(parse_swiper(params, section));
		}

		// latest updates
		if let Some(section) = html.select_first("#main-content") {
			components.push(HomeComponent {
				title: section.select(".cat-heading").and_then(|e| e.text()),
				value: HomeComponentValue::MangaChapterList {
					page_size: None,
					entries: section
						.select(".item")
						.map(|els| {
							els.take(10) // limit to 10, since that's as much as the page displays initially
								.filter_map(|e| {
									let link_href =
										e.select_first("a.manga-poster")?.attr("href")?;
									let chapter_link = e.select_first(".fd-list .chapter a")?;
									let chapter_link_href = chapter_link.attr("href")?;
									Some(MangaWithChapter {
										manga: Manga {
											key: link_href
												.strip_prefix(params.base_url.as_ref())
												.map(|s| s.into())
												.unwrap_or(link_href),
											title: e.select_first(".manga-name")?.text()?,
											cover: e
												.select_first(".manga-poster img")
												.and_then(|img| img.img_attr()),
											..Default::default()
										},
										chapter: Chapter {
											key: chapter_link_href
												.strip_prefix(params.base_url.as_ref())
												.map(|s| s.into())
												.unwrap_or(chapter_link_href),
											chapter_number: chapter_link
												.text()?
												.chars()
												.filter(|c| c.is_ascii_digit() || *c == '.')
												.collect::<String>()
												.parse::<f32>()
												.ok(),
											..Default::default()
										},
									})
								})
								.collect()
						})
						.unwrap_or_default(),
					listing: None,
				},
				..Default::default()
			});
		}

		if let Some(sidebar_sections) = html.select("#main-sidebar > section") {
			for section in sidebar_sections {
				let is_ranked = section.select_first("#chart-today").is_some();
				let Some(elements) = section.select(if is_ranked {
					"#chart-today .featured-block-ul > ul > li"
				} else {
					".featured-block-ul > ul > li"
				}) else {
					continue;
				};
				if !elements.is_empty() {
					components.push(HomeComponent {
						title: section.select(".cat-heading").and_then(|e| e.text()),
						value: HomeComponentValue::MangaList {
							ranking: is_ranked,
							page_size: Some(5),
							entries: elements
								.filter_map(|e| {
									Some(
										Manga {
											key: e
												.select_first("a.manga-poster")?
												.attr("abs:href")?
												.strip_prefix(params.base_url.as_ref())?
												.into(),
											title: e.select_first(".manga-name")?.text()?,
											cover: e
												.select_first(".manga-poster img")
												.and_then(|img| img.img_attr()),
											..Default::default()
										}
										.into(),
									)
								})
								.collect(),
							listing: None,
						},
						..Default::default()
					});
				}
			}
		}

		if let Some(completed_section) =
			html.select_first("#main-wrapper > div.container > div > section")
		{
			components.push(parse_swiper(params, completed_section));
		}

		Ok(HomeLayout { components })
	}

	fn get_image_request(
		&self,
		params: &Params,
		url: String,
		_context: Option<PageContext>,
	) -> Result<Request> {
		Ok(Request::get(url)?.header("Referer", &format!("{}/", params.base_url)))
	}

	fn process_page_image(
		&self,
		_params: &Params,
		response: ImageResponse,
		context: Option<PageContext>,
	) -> Result<ImageRef> {
		let Some(context) = context else {
			return Ok(response.image);
		};
		let number = |key: &str| context.get(key).and_then(|value| value.parse::<u32>().ok());
		let (Some(slice), Some(slices)) = (number("slice"), number("slices")) else {
			return Ok(response.image);
		};
		if slices < 2 || slice >= slices {
			return Ok(response.image);
		}

		let width = response.image.width();
		let height = response.image.height() as u32;
		// integer rows: the app rounds a fractional rect, doubling or dropping a row at the seam
		let top = height * slice / slices;
		let bottom = height * (slice + 1) / slices;
		let (top, slice_height) = (top as f32, (bottom - top) as f32);

		// before the AidokuRunner fix in e44d774 a canvas shorter than the image drew nothing
		let mut canvas = Canvas::new(width, slice_height);
		canvas.copy_image(
			&response.image,
			Rect::new(0.0, top, width, slice_height),
			Rect::new(0.0, 0.0, width, slice_height),
		);
		Ok(canvas.get_image())
	}

	fn handle_deep_link(&self, params: &Params, url: String) -> Result<Option<DeepLinkResult>> {
		let Some(path) = url.strip_prefix(params.base_url.as_ref()) else {
			return Ok(None);
		};

		const READ_PATH: &str = "read/";

		if path.starts_with(READ_PATH) {
			// ex: https://mangareader.to/read/the-weakest-job-becomes-the-strongest-in-the-world-with-past-life-knowledge-67999/en/chapter-2
			let end = path.find('/').unwrap_or(path.len());
			let manga_key = &path[..end];
			Ok(Some(DeepLinkResult::Chapter {
				manga_key: manga_key.into(),
				key: path.into(),
			}))
		} else {
			// ex: https://mangareader.to/the-weakest-job-becomes-the-strongest-in-the-world-with-past-life-knowledge-67999
			Ok(Some(DeepLinkResult::Manga { key: path.into() }))
		}
	}
}
