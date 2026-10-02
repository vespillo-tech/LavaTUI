## What does this change?

<!-- A sentence or two. Link the issue it fixes, if there is one (e.g. "Fixes #12"). -->

## How did you check it?

<!-- e.g. ran it in Ghostty at 80×24 and 200×60, added a test, compared screenshots. -->

## Checklist

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] If it changes how the lamp or the screen looks: a screenshot, and the snapshots updated with `UPDATE_SNAPSHOTS=1 cargo test` (diff reviewed)
- [ ] If it adds or changes a key or a setting: the README says so
