# Contributing to nutflix-eltor

Thanks for taking an interest. nutflix-eltor delivers video over paid El Tor circuits; start with the
[README](README.md) and [ADR 0001](docs/decisions/0001-architecture.md).

## Ground rules

- By contributing you agree your work is licensed under **AGPL-3.0-or-later**, the same licence as the project.
  There is no CLA and copyright stays with you.
- Be civil. See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
- Never commit secrets: control passwords, payment plans, Lightning credentials (runes, macaroons, seeds), private
  keys, `.env` files. `.gitignore` blocks the common cases, but it is not a safety net you should rely on.

## Workflow

1. Open an issue first for anything non-trivial, so design discussion happens before the code exists. Issues and
   pull requests live on the Nostr source of truth (see the README); the GitHub mirror accepts them too.
2. A design change is an edit to an ADR in `docs/decisions/` before it is code.
3. Branch off `main` and keep commits focused. Conventional Commits (`feat:`, `fix:`, `docs:` …).
4. Network tests (anything that starts tor, Lightning or a lab) run on your own machine or ours, never on hosted CI
   runners. Hosted CI builds and runs unit tests only.
5. Open a pull request against `main`. Explain *why*, not just *what*.
