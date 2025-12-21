# Contributing to IvoryValley

## Development Workflow

This project follows **Test-Driven Development (TDD)**. All contributors, including AI agents, **MUST** follow the TDD workflow.

### TDD Workflow (MANDATORY)

1. **Write tests first** - Before implementing any feature or fix, write failing tests that define the expected behavior
2. **Run the tests** - Verify that the new tests fail (red phase)
3. **Implement the code** - Write the minimum code needed to make the tests pass
4. **Run the tests again** - Verify that all tests pass (green phase)
5. **Refactor** - Clean up the code while keeping all tests passing

### For AI Agents

**CRITICAL**: AI agents working on this codebase MUST:

1. **Always write tests before implementation** - Never implement a feature without first writing tests for it
2. **Run `cargo test` after every change** - Verify tests pass before committing
3. **Follow the existing test patterns** - Look at existing tests in `tests/` and `src/*/tests` for guidance
4. **Use the test helpers** - Utilize utilities in `tests/common/mod.rs`

Example workflow for a new feature:

```rust
// 1. First, write the test (in tests/feature_test.rs or src/module.rs)
#[test]
fn test_new_feature() {
    let result = new_feature(input);
    assert_eq!(result, expected);
}

// 2. Run cargo test - it should fail
// 3. Implement the feature in src/
// 4. Run cargo test - it should pass
// 5. Refactor if needed, keeping tests green
```

## Test Structure

### Unit Tests

Unit tests live alongside the code they test, in a `tests` submodule:

```rust
// In src/module.rs
pub fn my_function() -> i32 {
    42
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_function() {
        assert_eq!(my_function(), 42);
    }
}
```

### Integration Tests

Integration tests live in the `tests/` directory:

- `tests/common/mod.rs` - Shared test utilities and helpers
- `tests/fixtures/` - Test data files
- `tests/*_test.rs` - Integration test files

### Async Tests

Use `#[tokio::test]` for async tests:

```rust
#[tokio::test]
async fn test_async_operation() {
    let result = async_function().await;
    assert!(result.is_ok());
}
```

## Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_name

# Run tests in a specific file
cargo test --test proxy_test
```

## Quality Checks

Before submitting a PR, ensure:

```bash
# All tests pass
cargo test

# No clippy warnings
cargo clippy --all-features -- -D warnings

# Code is formatted
cargo fmt --all -- --check
```

## Test Dependencies

The following test utilities are available:

- `tokio-test` - Async testing utilities
- `tempfile` - Temporary directories and files
- `axum-test` - HTTP/API testing helpers
- `pretty_assertions` - Better assertion output

## Continuous Integration

All PRs are automatically tested via GitHub Actions. The CI runs:

1. `cargo test` - All tests must pass
2. `cargo clippy` - No warnings allowed
3. `cargo fmt --check` - Code must be formatted
