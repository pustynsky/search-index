pub struct InventoryManager {
    stock_level: usize,
}

impl InventoryManager {
    pub fn reserve_inventory(&self, order_id: &str) -> Result<(), InventoryReservationError> {
        if self.stock_level == 0 || order_id.is_empty() {
            return Err(InventoryReservationError);
        }
        Ok(())
    }

    pub fn release_inventory(&mut self) {
        self.stock_level += 1;
    }
}

pub struct InventoryReservationError;
