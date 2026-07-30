pub struct OrderProcessor {
    inventory_manager: InventoryManager,
    retry_policy: RetryPolicy,
}

impl OrderProcessor {
    pub async fn process_order_async(&self, order_id: &str) -> Result<(), OrderError> {
        self.validate_order(order_id)?;
        self.retry_policy.execute_with_retry(order_id).await?;
        self.inventory_manager.reserve_inventory(order_id)?;
        Ok(())
    }

    fn validate_order(&self, order_id: &str) -> Result<(), OrderError> {
        if order_id.is_empty() {
            return Err(OrderError::new("order validation failed"));
        }
        Ok(())
    }
}

pub struct InventoryManager;
pub struct RetryPolicy;
pub struct OrderError;
