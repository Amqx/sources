use aidoku::{
	alloc::{String, Vec},
	imports::{
		defaults::defaults_get,
		net::{TimeUnit, set_rate_limit},
	},
};

const LANGUAGES_KEY: &str = "languages";
const RATE_LIMIT_KEY: &str = "rateLimit";
const TYPE_KEY: &str = "type";
const STATUS_KEY: &str = "status";
const EXCLUDE_GENRES_KEY: &str = "excludeGenres";

pub const LANGUAGES: [(&str, &str); 7] = [
	("en", "EN"),
	("fr", "FR"),
	("ja", "JA"),
	("pt-BR", "PT-BR"),
	("pt", "PT"),
	("es-419", "ES-LA"),
	("es", "ES"),
];

/// The languages to load chapters for, as `(aidoku code, site code)` pairs.
pub fn languages() -> Vec<(&'static str, &'static str)> {
	let selected = defaults_get::<Vec<String>>(LANGUAGES_KEY).unwrap_or_default();
	if selected.is_empty() {
		return LANGUAGES.to_vec();
	}
	LANGUAGES
		.into_iter()
		.filter(|(code, _)| selected.iter().any(|lang| lang == code))
		.collect()
}

/// Apply the user's request pacing.
pub fn apply_rate_limit() {
	let (permits, period) = match defaults_get::<String>(RATE_LIMIT_KEY).as_deref() {
		Some("fast") => (2, 1),
		Some("normal") => (1, 1),
		_ => (1, 2),
	};
	set_rate_limit(permits, period, TimeUnit::Seconds);
}

/// The type ("platform") the browse listings are pinned to, if any.
pub fn default_type() -> String {
	defaults_get::<String>(TYPE_KEY).unwrap_or_default()
}

/// The publication status the browse listings are pinned to, if any.
pub fn default_status() -> String {
	defaults_get::<String>(STATUS_KEY).unwrap_or_default()
}

/// Genre ids to exclude from every browse and search request.
pub fn excluded_genres() -> Vec<String> {
	defaults_get::<Vec<String>>(EXCLUDE_GENRES_KEY).unwrap_or_default()
}
