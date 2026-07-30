pub struct ConfigLoader;

pub struct RuntimeSettings {
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub cache_entry_ttl_seconds: u64,
    pub token_issuer: String,
    pub inventory_endpoint: String,
}

impl ConfigLoader {
    pub fn load_runtime_settings(&self, source: &str) -> Result<RuntimeSettings, ConfigError> {
        if source.is_empty() {
            return Err(ConfigError::new("config parse failed"));
        }
        todo!()
    }
}

pub struct ConfigError;
