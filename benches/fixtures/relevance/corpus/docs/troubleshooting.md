# Troubleshooting

- "order validation failed": verify that the order identifier is present.
- "config parse failed": validate MaxRetries and RetryDelayMs values.
- "retry budget exceeded": inspect the retry policy and service health.
- "service unavailable": verify the InventoryEndpoint setting.
- "session token expired": issue a new token through the authentication pipeline.
