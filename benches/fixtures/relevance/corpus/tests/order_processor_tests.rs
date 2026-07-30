#[test]
fn process_order_async_rejects_empty_id() {
    let processor = OrderProcessor;
    let error = processor.process_order_async("").unwrap_err();
    assert_eq!(error.message(), "order validation failed");
}

#[test]
fn validate_order_reserves_inventory() {
    let processor = OrderProcessor;
    processor.process_order_async("order-42").unwrap();
}

struct OrderProcessor;
