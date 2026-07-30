pub struct RetryPolicy {
    pub max_retries: u32,
    pub retry_delay_ms: u64,
}

impl RetryPolicy {
    pub async fn execute_with_retry(&self, operation_id: &str) -> Result<(), RetryBudgetExceeded> {
        for attempt in 0..self.max_retries {
            if self.try_operation(operation_id, attempt).await {
                return Ok(());
            }
        }
        Err(RetryBudgetExceeded::new("retry budget exceeded"))
    }

    async fn try_operation(&self, _operation_id: &str, _attempt: u32) -> bool {
        false
    }
}

pub struct RetryBudgetExceeded;
