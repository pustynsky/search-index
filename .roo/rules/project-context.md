# Project Rules

## Post-Change Checklist

After every code change, before completing the task, verify ALL of the following:

1. **Unit tests** — the change has test(s) covering the new/modified behavior
2. **E2E test plan** — `docs/e2e-test-plan.md` is updated with test scenarios for the change
3. **E2E test script** — evaluate whether `e2e-test.ps1` should also get new test cases for the change (CLI-testable scenarios). If yes — add them
4. **Documentation** — `README.md` and the rest relevant GIT-tracked documents are updated
5. **Neutral names** — all class/method names in docs, tests, and tool descriptions are generic (e.g., `UserService`, `OrderProcessor`) — never expose internal/proprietary names
6. **All tests pass** — run `cargo test --bin search` and confirm 0 failures
7. **Ask user to stop MCP server** — before reinstalling the binary, ask the user to stop the MCP server (restart VS Code or stop the search-index server)
8. **Reinstall binary** — `cargo install --path . --force`
9. **Run E2E tests** — after the binary is installed, run `.\e2e-test.ps1` and confirm 0 failures

## Git Workflow — After All Tests Pass

After all tests pass and the binary is reinstalled, propose creating a branch and committing:

1. **Ask user** — "Would you like to create a branch and commit these changes?"
2. If yes:
   - Check current branch with `git rev-parse --abbrev-ref HEAD`
   - If on `main`: run `git pull` then `git checkout -b <branch-name>`
   - If NOT on `main`: run `git stash`, `git checkout main`, `git pull`, `git checkout -b <branch-name>`, `git stash pop`
   - Branch name format: `users/<user-alias>/<feature-name>`
3. **Stage tracked changes only** — `git add -u` (never auto-add untracked files)
4. **Prepare commit message** — write a concise commit title
5. **Prepare PR description** — write a detailed description of all changes in Markdown format
6. **Write PR description to file** — save the PR description to `docs/pr-description.md` so the user can copy it easily (this file is NOT tracked in git — it's a temp artifact)
7. **Ask user to commit manually** — present the commit title + PR description and let the user do `git commit` themselves

## Environment Rules

- **Windows environment** — this project runs on Windows (cmd / PowerShell). Never use Unix-only commands like `tail`, `head`, `grep`, `sed`, `awk`, `wc`. Use PowerShell equivalents or native Rust/cargo commands instead.
- **Testing is mandatory** — every code change MUST include:
  - **Unit tests** covering the new/modified behavior
  - **E2E test plan update** (`docs/e2e-test-plan.md`) with a test scenario for the change
  - **E2E test script update** (`e2e-test.ps1`) if the change is CLI-testable
- **Never skip tests** — even for "internal" optimizations or refactors. If the behavior is testable, add tests.

## Git Rules

- **Tracked files only** — when committing to branches (via `commit_and_push`, `git add`, or any other tool), always stage only tracked (modified) files. Never auto-add untracked files. Use `git add -u` / `includeUntrackedFiles: false`. Untracked files must be added explicitly by the user.