use aidoku::{
	alloc::{String, Vec},
	imports::defaults::defaults_get,
};

fn languages(key: &str) -> Vec<String> {
	defaults_get::<String>(key)
		.unwrap_or_default()
		.split(',')
		.map(|language| language.trim().to_lowercase())
		.filter(|language| !language.is_empty())
		.collect()
}

pub fn language_allowed(language: &str) -> bool {
	let language = language.trim().to_lowercase();
	let blocked = languages("languagesBlacklist");
	if blocked.iter().any(|blocked| blocked == &language) {
		return false;
	}

	let allowed = languages("languagesWhitelist");
	allowed.is_empty()
		|| language == "unknown"
		|| allowed.iter().any(|allowed| allowed == &language)
}
