export interface OrderRequest {
    orderId: string;
    inventoryEndpoint: string;
}

export class OrderClient {
    constructor(private readonly retryDelayMs: number) {}

    async submitOrder(request: OrderRequest): Promise<void> {
        if (!request.orderId) {
            throw new Error("service unavailable");
        }
        await this.sendOrder(request);
    }

    private async sendOrder(_request: OrderRequest): Promise<void> {
        await Promise.resolve(this.retryDelayMs);
    }
}
