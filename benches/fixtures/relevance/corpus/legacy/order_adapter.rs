pub struct LegacyOrderAdapter {
    order_processor: OrderProcessor,
}

impl LegacyOrderAdapter {
    pub async fn process_order(&self, order_id: &str) -> Result<(), LegacyError> {
        self.order_processor.process_order_async(order_id).await
    }

    pub async fn submit_order(&self, order_id: &str) -> Result<(), LegacyError> {
        self.process_order(order_id).await
    }
}

pub struct OrderProcessor;
pub struct LegacyError;
