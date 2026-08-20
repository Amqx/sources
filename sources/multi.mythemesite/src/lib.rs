#![no_std]
use aidoku::{AidokuError, DeepLinkResult, HomeLayout, Result, Source, alloc::String, prelude::*};
use mytheme::{Impl, MyTheme, Params};

struct MyThemeSite;

impl Impl for MyThemeSite {
	fn new() -> Self {
		Self
	}

	fn params(&self) -> Params {
		Params::default()
	}

	fn get_home(&self, _params: &Params) -> Result<HomeLayout> {
		Err(AidokuError::Unimplemented)
	}

	fn handle_deep_link(&self, _params: &Params, _url: String) -> Result<Option<DeepLinkResult>> {
		Err(AidokuError::Unimplemented)
	}
}

register_source!(MyTheme<MyThemeSite>, ListingProvider, Home, DeepLinkHandler);
