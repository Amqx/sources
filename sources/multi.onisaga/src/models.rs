use aidoku::alloc::{String, Vec};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct LivewireRequest<'a, U> {
	#[serde(rename = "_token")]
	pub token: &'a str,
	pub components: [Component<'a, U>; 1],
}

#[derive(Serialize)]
pub struct Component<'a, U> {
	pub snapshot: &'a str,
	pub updates: U,
	pub calls: [Call<'a>; 1],
}

#[derive(Serialize)]
pub struct Call<'a> {
	#[serde(rename = "type")]
	pub kind: &'a str,
	pub path: &'a str,
	pub method: &'a str,
	pub params: Vec<String>,
}

impl<'a> Call<'a> {
	pub fn new(method: &'a str, params: Vec<String>) -> Self {
		Self {
			kind: "call",
			path: "",
			method,
			params,
		}
	}
}

#[derive(Deserialize, Default)]
pub struct LivewireResponse {
	#[serde(default)]
	pub components: Vec<ComponentResponse>,
}

#[derive(Deserialize, Default)]
pub struct ComponentResponse {
	#[serde(default)]
	pub effects: Effects,
	#[serde(default)]
	pub snapshot: String,
}

#[derive(Deserialize, Default)]
pub struct Effects {
	pub html: Option<String>,
}

/// The `post-filter` component's public properties.
#[derive(Serialize, PartialEq)]
pub struct PostFilterUpdates {
	pub platform: String,
	pub status: String,
	pub sort: String,
	#[serde(rename = "min_chapters")]
	pub min_chapters: String,
	pub group: Option<String>,
	#[serde(rename = "release_start")]
	pub release_start: Option<String>,
	#[serde(rename = "release_end")]
	pub release_end: Option<String>,
	pub genre: Vec<String>,
	#[serde(rename = "excludeGenre")]
	pub exclude_genre: Vec<String>,
}

impl Default for PostFilterUpdates {
	fn default() -> Self {
		Self {
			platform: String::new(),
			status: String::new(),
			sort: String::from("created_at"),
			min_chapters: String::new(),
			group: None,
			release_start: None,
			release_end: None,
			genre: Vec::new(),
			exclude_genre: Vec::new(),
		}
	}
}

impl PostFilterUpdates {
	pub fn is_default(&self) -> bool {
		*self == Self::default()
	}
}

/// The `manga.chapter-list` component's public properties.
#[derive(Serialize)]
pub struct ChapterListUpdates<'a> {
	pub language: &'a str,
}

/// One page's signed image url, from `GET /api/chapter/{id}/page/{order}`.
#[derive(Deserialize, Default)]
pub struct PageApiResponse {
	pub url: Option<String>,
	pub message: Option<String>,
}
