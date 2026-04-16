# orb-relay-messages

## Pre-push checklist

After making a commit and before pushing, run formatting, clippy with maximum feature coverage, and the full workspace test suite:

```sh
cargo fmt --all
cargo clippy --workspace --tests --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```
