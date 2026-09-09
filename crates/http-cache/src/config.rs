use std::time::Duration;

use super::SCHEMA_VERSION;

const MAX_ENTRY_SIZE: usize = 8 * 1024 * 1024;
const STALE_RETENTION: Duration = Duration::from_hours(24);
const KEY_PREFIX: &str = "alternate-http-cache";

#[derive(Debug, Clone)]
pub struct HttpCacheConfig {
    pub key_prefix: String,
    pub cache_authorized_requests: bool,
    pub cache_set_cookie_responses: bool,
    pub max_entry_size: Option<usize>,
    pub stale_retention: Duration,
}

impl HttpCacheConfig {
    pub fn builder() -> HttpCacheConfigBuilder {
        HttpCacheConfigBuilder::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct HttpCacheConfigBuilder {
    key_prefix: String,
    cache_authorized_requests: bool,
    cache_set_cookie_responses: bool,
    #[allow(clippy::option_option)]
    max_entry_size: Option<Option<usize>>,
    stale_retention: Option<Duration>,
}

impl HttpCacheConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn key_prefix(mut self, value: impl Into<String>) -> Self {
        self.key_prefix = value.into();
        self
    }

    #[must_use]
    pub fn cache_authorized_requests(mut self, value: bool) -> Self {
        self.cache_authorized_requests = value;
        self
    }

    #[must_use]
    pub fn cache_set_cookie_responses(mut self, value: bool) -> Self {
        self.cache_set_cookie_responses = value;
        self
    }

    #[must_use]
    pub fn max_entry_size(mut self, value: Option<usize>) -> Self {
        self.max_entry_size = Some(value);
        self
    }

    #[must_use]
    pub fn stale_retention(mut self, value: Duration) -> Self {
        self.stale_retention = Some(value);
        self
    }

    pub fn build(self) -> HttpCacheConfig {
        HttpCacheConfig {
            key_prefix: if self.key_prefix.is_empty() {
                format!("{KEY_PREFIX}:v{SCHEMA_VERSION}")
            } else {
                self.key_prefix
            },
            cache_authorized_requests: self.cache_authorized_requests,
            cache_set_cookie_responses: self.cache_set_cookie_responses,
            max_entry_size: self.max_entry_size.unwrap_or(Some(MAX_ENTRY_SIZE)),
            stale_retention: self.stale_retention.unwrap_or(STALE_RETENTION),
        }
    }
}
