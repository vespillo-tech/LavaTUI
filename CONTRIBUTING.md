# Contributing to LavaTUI

Thanks for wanting to help! Bug reports, ideas and code are all welcome.

## Reporting a bug or asking for a feature

Open an issue and pick a template. For bugs, tell us your terminal app,
your operating system and roughly how big the window was, and add a
screenshot if you can: the lamp looks different in every terminal.

## Working on the code

You need [Rust](https://rustup.rs) (stable). Then:

```bash
cargo run --release        # run the lamp (release mode: the simulation needs the speed)
cargo test                 # run the tests
```

Before you open a pull request, please make sure these three pass. CI runs
them on macOS, Linux and Windows:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

A few things that help a change land quickly:

- **Keep it small.** One fix or one feature per pull request.
- **Looks matter.** If your change shows up on screen, try it at a small and
  a large window size, and include a screenshot.
- **Snapshots.** Some tests compare whole drawn frames with saved text files.
  If you changed the picture on purpose, rewrite them with
  `UPDATE_SNAPSHOTS=1 cargo test` and check the diff before committing.
- **Adding a style, clock face, widget or key?** `CLAUDE.md` ("Adding
  things") lists the few places each one touches, and `docs/design.md`
  describes how the screen is laid out.

## License

LavaTUI is dual-licensed under MIT or Apache-2.0, at your option. Unless you
say otherwise, anything you contribute is licensed the same way, with no
extra terms.
