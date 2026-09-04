use aidoku::alloc::{String, collections::BTreeMap};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SearchResponse {
	pub data: BTreeMap<String, SearchBook>,
}

#[derive(Deserialize)]
pub struct SearchBook {
	pub value: String,
}

#[derive(Deserialize)]
pub struct ChapterPagesResponse {
	pub src: String,
}
