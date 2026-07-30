pub struct CacheStore {
    pub cache_entry_ttl: u64,
}

impl CacheStore {
    pub fn get_cached_order(&self, order_id: &str) -> Result<String, CacheMiss> {
        if order_id.is_empty() {
            return Err(CacheMiss);
        }
        Ok(order_id.to_string())
    }

    pub fn evict_expired_entries(&mut self) {
        let _ttl = self.cache_entry_ttl;
    }
}

pub struct CacheMiss;
