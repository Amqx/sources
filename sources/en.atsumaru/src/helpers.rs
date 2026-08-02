use aidoku::{
	ContentRating, FilterValue, MangaStatus, Result, Viewer,
	alloc::{String, Vec, borrow::ToOwned, format, string::ToString},
	bail,
	helpers::uri::QueryParameters,
	imports::{
		defaults::defaults_get,
		net::{Request, Response},
	},
};
use serde::de::DeserializeOwned;

pub const BASE_URL: &str = "https://atsu.moe";

/// Static files are served from a separate host. Requesting them from the main
/// one only earns a permanent redirect here.
const CDN_URL: &str = "https://cdn.atsu.moe";

/// The manga types the website browses with. It always requests all of them.
const TYPES: &str = "Manga,Manwha,Manhua,OEL";

/// The amount of search results requested per page.
pub const PER_PAGE: i32 = 40;

/// Typesense sort fields, indexed by the position of the option in the sort
/// filter. The first one is empty because "Relevance" is the search engine's
/// own ordering, which is requested by leaving `sort_by` off entirely.
const SORT_FIELDS: [&str; 7] = [
	"",
	"views",
	"trending",
	"dateAdded",
	"released",
	// the search index stores ratings under a different name than manga pages do
	"mbRating",
	"title",
];

/// Whether the user enabled adult mode. The website uses this as a switch
/// between safe and +18 content rather than as an "include adult" toggle.
pub fn adult_mode() -> bool {
	defaults_get::<bool>("adultMode").unwrap_or(false)
}

pub fn api_get(url: &str) -> Result<Request> {
	Ok(Request::get(url)?
		.header("Accept", "*/*")
		.header("Referer", &format!("{BASE_URL}/"))
		.header("Content-Type", "application/json"))
}

pub fn api_json<T: DeserializeOwned>(url: &str) -> Result<T> {
	response_json(api_get(url)?.send()?)
}

/// Reads a json response, failing on error codes. The api answers those with a
/// json body of its own, which would otherwise parse into an empty result and
/// leave the user looking at a page that seems merely empty.
pub fn response_json<T: DeserializeOwned>(response: Response) -> Result<T> {
	if response.status_code() >= 400 {
		bail!("Response Error: {}", response.status_code())
	}
	response.get_json_owned()
}

/// The url of one of the infinite scroll listing endpoints, which are paginated
/// starting from zero.
pub fn listing_url(endpoint: &str, page: i32) -> String {
	let adult = if adult_mode() { "&adult=1" } else { "" };
	format!(
		"{BASE_URL}/api/infinite/{endpoint}?page={}&types={TYPES}{adult}",
		page - 1
	)
}

/// The url of the Typesense search collection, with every filter applied.
pub fn search_url(query: Option<&str>, page: i32, filters: &[FilterValue]) -> String {
	let mut sort_index = 0usize;
	let mut ascending = false;
	let mut genres_included: &[String] = &[];
	let mut genres_excluded: &[String] = &[];
	let mut tags_included: &[String] = &[];
	let mut tags_excluded: &[String] = &[];
	let mut types: &[String] = &[];
	let mut statuses: &[String] = &[];
	let mut year = (None, None);
	let mut chapter_count = (None, None);
	let mut official_only = false;

	for filter in filters {
		match filter {
			FilterValue::Sort {
				index,
				ascending: a,
				..
			} => {
				sort_index = usize::try_from(*index).unwrap_or(0);
				ascending = *a;
			}
			FilterValue::MultiSelect {
				id,
				included,
				excluded,
			} => match id.as_str() {
				"genre" => {
					genres_included = included;
					genres_excluded = excluded;
				}
				"tag" => {
					tags_included = included;
					tags_excluded = excluded;
				}
				"type" => types = included,
				"status" => statuses = included,
				_ => {}
			},
			FilterValue::Range { id, from, to } => match id.as_str() {
				"year" => year = (*from, *to),
				"chapters" => chapter_count = (*from, *to),
				_ => {}
			},
			FilterValue::Check { id, value } if id == "official" => official_only = *value == 1,
			_ => {}
		}
	}

	let mut clauses: Vec<String> = Vec::new();
	clauses.push("hidden:!=true".to_owned());

	// Included values are separate clauses so that they're all required, while
	// excluded values are a single "not any of" clause.
	for id in genres_included {
		clauses.push(format!("genreIds:=`{id}`"));
	}
	if !genres_excluded.is_empty() {
		clauses.push(format!("genreIds:!=[{}]", quoted_list(genres_excluded)));
	}
	for id in tags_included {
		clauses.push(format!("tagIds:=`{id}`"));
	}
	if !tags_excluded.is_empty() {
		clauses.push(format!("tagIds:!=[{}]", quoted_list(tags_excluded)));
	}
	if !types.is_empty() {
		clauses.push(format!("type:=[{}]", quoted_list(types)));
	}
	if !statuses.is_empty() {
		clauses.push(format!("status:=[{}]", quoted_list(statuses)));
	}
	if let Some(from) = year.0 {
		clauses.push(format!("releaseYear:>={}", from as i32));
	}
	if let Some(to) = year.1 {
		clauses.push(format!("releaseYear:<={}", to as i32));
	}
	if let Some(from) = chapter_count.0 {
		clauses.push(format!("chapterCount:>={}", from as i32));
	}
	if let Some(to) = chapter_count.1 {
		clauses.push(format!("chapterCount:<={}", to as i32));
	}
	if !adult_mode() {
		clauses.push("isAdult:=false".to_owned());
	}
	if official_only {
		clauses.push("officialTranslation:=true".to_owned());
	}
	clauses.push(
		"(mbContentRating:=[`Safe`,`Suggestive`,`Erotica`] || mbContentRating:!=*)".to_owned(),
	);
	clauses.push("views:>0".to_owned());

	let query = query.filter(|q| !q.is_empty());

	let mut qs = QueryParameters::new();
	qs.push("q", Some(query.unwrap_or("*")));
	qs.push("filter_by", Some(&clauses.join(" && ")));

	let sort_field = SORT_FIELDS[sort_index.min(SORT_FIELDS.len() - 1)];
	if !sort_field.is_empty() {
		let direction = if ascending { "asc" } else { "desc" };
		qs.push("sort_by", Some(&format!("{sort_field}:{direction}")));
	} else if query.is_none() {
		// relevance is meaningless without a search term, so fall back to views
		qs.push("sort_by", Some("views:desc"));
	}

	if query.is_some() {
		qs.push("query_by", Some("title,englishTitle,otherNames,authors"));
		qs.push("query_by_weights", Some("4,3,2,1"));
		qs.push("num_typos", Some("4,3,2,1"));
	}

	qs.push("page", Some(&page.to_string()));
	qs.push("per_page", Some(&PER_PAGE.to_string()));

	format!("{BASE_URL}/collections/manga/documents/search?{qs}")
}

fn quoted_list(ids: &[String]) -> String {
	ids.iter()
		.map(|id| format!("`{id}`"))
		.collect::<Vec<String>>()
		.join(",")
}

/// Turns a path returned by the api into an absolute image url. Paths can come
/// through as absolute urls, protocol relative urls, or as a path relative to
/// the static file directory, with or without a leading slash.
pub fn image_url(path: &str) -> String {
	if path.starts_with("//") {
		return force_https(&format!("https:{path}"));
	}
	if path.starts_with("http") {
		return force_https(path);
	}
	let path = path.strip_prefix('/').unwrap_or(path);
	let path = path.strip_prefix("static/").unwrap_or(path);
	format!("{CDN_URL}/static/{path}")
}

/// Upgrades a url to https, also repairing the colon-less protocols that some
/// entries are stored with.
fn force_https(url: &str) -> String {
	for prefix in ["http://", "https://", "http//", "https//"] {
		if let Some(rest) = url.strip_prefix(prefix) {
			return format!("https://{rest}");
		}
	}
	url.to_owned()
}

pub fn parse_status(status: Option<&str>) -> MangaStatus {
	match status.map(|s| s.trim().to_lowercase()).as_deref() {
		Some("ongoing") => MangaStatus::Ongoing,
		Some("completed") => MangaStatus::Completed,
		Some("hiatus") => MangaStatus::Hiatus,
		Some("canceled") | Some("cancelled") => MangaStatus::Cancelled,
		_ => MangaStatus::Unknown,
	}
}

pub fn parse_viewer(kind: Option<&str>) -> Viewer {
	match kind {
		Some("Manga") => Viewer::RightToLeft,
		Some("OEL") => Viewer::LeftToRight,
		Some("Manwha") | Some("Manhua") => Viewer::Webtoon,
		_ => Viewer::Unknown,
	}
}

/// The content rating of an entry. Only the adult flag is authoritative, so
/// genres are used to tell the remaining entries apart.
pub fn parse_content_rating(is_adult: bool, genres: &[String]) -> ContentRating {
	if is_adult {
		return ContentRating::NSFW;
	}
	let mut suggestive = false;
	for genre in genres {
		match genre.as_str() {
			"Hentai" | "Adult" | "Smut" => return ContentRating::NSFW,
			"Ecchi" | "Mature" | "Erotica" => suggestive = true,
			_ => {}
		}
	}
	if suggestive {
		ContentRating::Suggestive
	} else {
		ContentRating::Safe
	}
}

/// The rating string used by the api's `mbContentRating` field, which is only
/// present on search results.
pub fn parse_mb_content_rating(rating: Option<&str>) -> Option<ContentRating> {
	match rating {
		Some("Safe") => Some(ContentRating::Safe),
		Some("Suggestive") => Some(ContentRating::Suggestive),
		Some("Erotica") | Some("Pornographic") => Some(ContentRating::NSFW),
		_ => None,
	}
}

/// The year of a unix timestamp in milliseconds, counted a year at a time. The
/// input is clamped to years 1 through 9999 so that a nonsensical timestamp
/// can't turn the walk into a hang.
pub fn year_from_millis(millis: i64) -> i32 {
	const MIN_MILLIS: i64 = -62_135_596_800_000; // 0001-01-01
	const MAX_MILLIS: i64 = 253_402_300_799_000; // 9999-12-31

	const fn is_leap(year: i32) -> bool {
		year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
	}

	let mut days = millis.clamp(MIN_MILLIS, MAX_MILLIS).div_euclid(86_400_000);
	let mut year = 1970;
	while days < 0 {
		year -= 1;
		days += if is_leap(year) { 366 } else { 365 };
	}
	loop {
		let length = if is_leap(year) { 366 } else { 365 };
		if days < length {
			return year;
		}
		days -= length;
		year += 1;
	}
}

/// Formats a chapter number the way the website writes it in titles, so that
/// redundant chapter titles can be recognized.
fn number_string(number: f32) -> String {
	let whole = number as i64;
	if whole as f32 == number {
		format!("{whole}")
	} else {
		format!("{number}")
	}
}

/// Drops chapter titles that only repeat the chapter number, since Aidoku
/// already displays it.
pub fn clean_chapter_title(title: Option<String>, number: Option<f32>) -> Option<String> {
	let title = title?;
	let trimmed = title.trim();
	if trimmed.is_empty() {
		return None;
	}
	if let Some(number) = number {
		let number = number_string(number);
		let lowercase = trimmed.to_lowercase();
		let redundant = [
			number.clone(),
			format!("chapter {number}"),
			format!("ch {number}"),
			format!("ch. {number}"),
			format!("episode {number}"),
		]
		.contains(&lowercase);
		if redundant {
			return None;
		}
	}
	if trimmed.len() == title.len() {
		Some(title)
	} else {
		Some(trimmed.to_owned())
	}
}
