# Contributing

Thank you for considering a change to `tpplc`.

For a large change or a new protocol command, open an issue first. This lets
maintainers discuss the change before you write code.

Keep each change focused. Do not include private network data in issues or
pull requests. Remove adapter MAC addresses from logs unless they are needed
for the report.

Before you submit a change, run:

```sh
cargo fmt --check
cargo test
cargo clippy -- -D warnings
cargo build --release
```

For a hardware change, include the adapter model, hardware version, firmware,
operating system, interface type, and test result. State if you tested the
change with an adapter or with a software test only.

## AI-assisted contributions

Contributors may use AI tools. Each contributor is responsible for every
change they submit.

Before submission:

- Review the change.
- Understand the change.
- Build and test the change.
- Disclose substantial AI-generated code or documentation in the pull request.

Do not submit unreviewed AI output.
