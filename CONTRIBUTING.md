# Contributing to llm-sync

Thanks for helping. Bug reports, small fixes and new examples are all welcome.

## Reporting a problem

Open an issue at https://gitlab.com/mattbusel/llm-sync/-/issues and pick the
"Bug" template. The most useful report has the crate version, your Rust version
(`rustc --version`), the smallest code that shows the problem, and what you
expected instead.

## Making a change

1. Fork the project on GitLab and create a branch.
2. Make the change, with a test that fails without it.
3. Run the same checks CI runs:

   ```bash
   cargo test --all-features
   cargo clippy --all-targets --all-features -- -D warnings
   RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
   ```

4. Add a line to the `Unreleased` section of `CHANGELOG.md` (create it if it
   is not there) saying what changed for users.
5. Open a merge request. Say what the change fixes or adds and how you tested it.

## Ground rules

- Library code must not panic on user input: return an error instead.
  `unwrap`, `expect` and `panic!` are for tests only.
- Every public item needs a doc comment.
- Keep the default feature set small. Anything that pulls in a large or
  platform-specific dependency goes behind a cargo feature.
- Plain language in docs and errors: say what happened and what to do.

By contributing you agree that your work is released under the MIT license
in `LICENSE`.
