# AGENTS.md — Coding agent guidance for dir2web

See [CONTRIBUTING.md](CONTRIBUTING.md). In particular:

## Before declaring a task done

1. `cargo fmt`
2. `cargo build`
3. `cargo test`
4. `cargo clippy --all-targets -- -D warnings`

## Commits

- Follow the Conventional Commits specification; releases and the changelog
  are generated from commit messages. Include your model as the last item in
  the subject line in parentheses.
- After changing dependencies, regenerate `THIRD-PARTY-LICENSES` as described
  in CONTRIBUTING.md.
