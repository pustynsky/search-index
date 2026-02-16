# Project Rules

## Post-Change Checklist

After every code change, before completing the task, verify ALL of the following:

1. **Unit tests** — the change has test(s) covering the new/modified behavior
2. **E2E tests** — `docs/e2e-test-plan.md` is updated with test scenarios for the change
3. **Documentation** — `README.md` and tool descriptions in code are updated
4. **Neutral names** — all class/method names in docs, tests, and tool descriptions are generic (e.g., `UserService`, `OrderProcessor`) — never expose internal/proprietary names
5. **All tests pass** — run `cargo test --bin search` and confirm 0 failures
6. **Ask user to stop MCP server** — before reinstalling the binary, ask the user to stop the MCP server (restart VS Code or stop the search-index server)
7. **Reinstall binary** — `cargo install --path . --force`