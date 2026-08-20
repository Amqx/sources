#![no_std]
use aidoku::{AidokuError, DeepLinkResult, HomeLayout, Result, Source, alloc::String, prelude::*};
use madara::{Impl, Madara, Params};

struct MadaraTest;

impl Impl for MadaraTest {
	fn new() -> Self {
		Self
	}

	fn params(&self) -> Params {
		Params {
			..Default::default()
		}
	}

	fn get_home(&self, _params: &Params) -> Result<HomeLayout> {
		Err(AidokuError::Unimplemented)
	}

	fn handle_deep_link(&self, _params: &Params, _url: String) -> Result<Option<DeepLinkResult>> {
		Err(AidokuError::Unimplemented)
	}
}

register_source!(Madara<MadaraTest>, ListingProvider, Home, DeepLinkHandler);
