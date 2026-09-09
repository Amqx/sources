use aidoku::{
	alloc::String,
	imports::{html::Element, net::Request},
	prelude::*,
};

// enough to reach the frame header unless the file leads with a large colour profile
const HEADER_BYTES: usize = 16 * 1024;
// the deepest stacked image measured holds 57 pages; past this the header was misread
const STACKED_PAGE_LIMIT: u32 = 64;

pub trait ElementImageAttr {
	fn img_attr(&self) -> Option<String>;
}

impl ElementImageAttr for Element {
	fn img_attr(&self) -> Option<String> {
		self.attr("abs:data-lazy-src")
			.or_else(|| self.attr("abs:data-src"))
			.or_else(|| self.attr("abs:data-url"))
			.or_else(|| self.attr("abs:src"))
			.or_else(|| self.attr("data-url"))
	}
}

pub fn stacked_page_count(url: &str, page_ratio: f32) -> u32 {
	let Some((width, height)) = image_size(url) else {
		return 1;
	};
	slice_count(width, height, page_ratio)
}

fn image_size(url: &str) -> Option<(u32, u32)> {
	let range = format!("bytes=0-{}", HEADER_BYTES - 1);
	let head = Request::get(url)
		.ok()?
		.header("Range", range.as_str())
		.data()
		.ok()?;
	jpeg_size(&head)
}

pub fn slice_count(width: u32, height: u32, page_ratio: f32) -> u32 {
	let page_height = width as f32 * page_ratio;
	let pages = height as f32 / page_height;
	let rounded = (pages + 0.5) as u32;
	if !(2..=STACKED_PAGE_LIMIT).contains(&rounded) {
		return 1;
	}
	let neighbour = if pages < rounded as f32 {
		rounded - 1
	} else {
		rounded + 1
	};
	for count in [rounded, neighbour] {
		if (2..=STACKED_PAGE_LIMIT).contains(&count) && height.is_multiple_of(count) {
			return count;
		}
	}
	rounded
}

// stacked images are jpeg: webp caps a side at 16383 pixels, too short to stack a chapter into
fn jpeg_size(head: &[u8]) -> Option<(u32, u32)> {
	fn length(head: &[u8], at: usize) -> Option<usize> {
		Some(usize::from(u16::from_be_bytes([
			*head.get(at)?,
			*head.get(at + 1)?,
		])))
	}

	if *head.first()? != 0xFF || *head.get(1)? != 0xD8 {
		return None;
	}

	let mut index = 2;
	while *head.get(index)? == 0xFF {
		match *head.get(index + 1)? {
			// padding ahead of the next marker
			0xFF => index += 1,
			// markers with no segment behind them
			0x01 | 0xD0..=0xD9 => index += 2,
			// frame header: precision, then the size
			0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
				let height = length(head, index + 5)?.try_into().ok()?;
				let width = length(head, index + 7)?.try_into().ok()?;
				return Some((width, height));
			}
			_ => index += 2 + length(head, index + 2)?,
		}
	}

	None
}
