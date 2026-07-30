namespace Example.Orders;

public sealed class OrderController
{
    private readonly OrderProcessor _orderProcessor;

    public OrderController(OrderProcessor orderProcessor)
    {
        _orderProcessor = orderProcessor;
    }

    public async Task ProcessOrderAsync(string orderId)
    {
        if (string.IsNullOrWhiteSpace(orderId))
        {
            throw new InvalidOperationException("order request rejected");
        }

        await _orderProcessor.ProcessOrderAsync(orderId);
    }
}
