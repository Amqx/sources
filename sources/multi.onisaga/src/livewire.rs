use aidoku::{
	Result,
	alloc::{String, Vec},
	imports::{
		html::Document,
		net::{Request, Response},
	},
	prelude::*,
};
use serde::Serialize;

use crate::{
	BASE_URL,
	models::{Call, Component, LivewireRequest, LivewireResponse},
};

pub struct State {
	pub snapshot: String,
	pub token: String,
}

impl State {
	/// Find the snapshot of the named component in a freshly loaded page.
	pub fn extract(body: &str, doc: &Document, component: &str) -> Result<Self> {
		let token = doc
			.select_first("meta[name=csrf-token]")
			.and_then(|el| el.attr("content"))
			.or_else(|| {
				doc.select_first("input[name=_token]")
					.and_then(|el| el.attr("value"))
			})
			.filter(|token| !token.is_empty())
			.ok_or(error!("Missing CSRF token"))?;

		let snapshot = find_snapshot(body, component)
			.ok_or(error!("Missing Livewire snapshot for {component}"))?;

		Ok(Self { snapshot, token })
	}
}

fn find_snapshot(body: &str, component: &str) -> Option<String> {
	const ATTR: &str = "wire:snapshot=\"";

	let mut rest = body;
	while let Some(idx) = rest.find(ATTR) {
		let value = &rest[idx + ATTR.len()..];
		let end = value.find('"')?;
		if value[..end].contains(component) {
			return Some(unescape_attr(&value[..end]));
		}
		rest = &value[end..];
	}
	None
}

/// Undo the escaping an attribute value carries.
fn unescape_attr(value: &str) -> String {
	value
		.replace("&quot;", "\"")
		.replace("&#34;", "\"")
		.replace("&apos;", "'")
		.replace("&#39;", "'")
		.replace("&lt;", "<")
		.replace("&gt;", ">")
		.replace("&amp;", "&")
}

/// Build a `POST /livewire/update` for one method call on one component.
pub fn request<U: Serialize>(
	token: &str,
	snapshot: &str,
	referer: &str,
	updates: U,
	method: &str,
	params: Vec<String>,
) -> Result<Request> {
	let body = serde_json::to_string(&LivewireRequest {
		token,
		components: [Component {
			snapshot,
			updates,
			calls: [Call::new(method, params)],
		}],
	})
	.map_err(|_| error!("Could not encode the Livewire request"))?;

	Ok(Request::post(format!("{BASE_URL}/livewire/update"))?
		.header("Content-Type", "application/json")
		.header("Accept", "application/json")
		.header("X-Livewire", "")
		.header("X-Requested-With", "XMLHttpRequest")
		.header("Origin", BASE_URL)
		.header("Referer", referer)
		.body(body))
}

/// The rendered fragment and the snapshot to send with the next call.
pub fn parse(response: Response) -> Option<(String, String)> {
	let mut dto = response.get_json_owned::<LivewireResponse>().ok()?;
	if dto.components.is_empty() {
		return None;
	}
	let component = dto.components.swap_remove(0);
	Some((component.effects.html?, component.snapshot))
}

/// Whether the paginator still offers a next page.
pub fn has_next_page(html: &str) -> bool {
	let mut from = 0;
	while let Some(offset) = html[from..].find("nextPage") {
		let idx = from + offset;
		let start = html[..idx].rfind('<').unwrap_or(0);
		let end = html[idx..].find('>').map_or(html.len(), |end| idx + end);
		if !html[start..end].contains("disabled") {
			return true;
		}
		from = end;
	}
	false
}

#[cfg(test)]
mod tests {
	use super::*;
	use aidoku_test::aidoku_test;

	#[aidoku_test]
	fn picks_the_named_component() {
		let body = concat!(
			r#"<div wire:snapshot="{&quot;memo&quot;:{&quot;name&quot;:&quot;navbar&quot;}}"></div>"#,
			r#"<div wire:snapshot="{&quot;memo&quot;:{&quot;name&quot;:&quot;post-filter&quot;}}"></div>"#,
		);
		assert_eq!(
			find_snapshot(body, "post-filter").as_deref(),
			Some(r#"{"memo":{"name":"post-filter"}}"#)
		);
		assert_eq!(find_snapshot(body, "manga.chapter-list"), None);
	}

	#[aidoku_test]
	fn reads_the_paginator() {
		assert!(has_next_page(
			r#"<button wire:click="nextPage">Next</button>"#
		));
		assert!(!has_next_page(
			r#"<button wire:click="nextPage" disabled>Next</button>"#
		));
		assert!(has_next_page(concat!(
			r#"<button wire:click="previousPage" disabled></button>"#,
			r#"<button wire:click="nextPage"></button>"#,
		)));
		assert!(!has_next_page("<div>no paginator</div>"));
	}
}
