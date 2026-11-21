# Contributing to Hive

Hive is open source under the MIT licence. Bug reports, fixes, docs and
ideas are welcome. This page says how to build Hive, how to check a change,
and how a change gets in.

## Build

You need Rust 1.95 or later ([rustup](https://rustup.rs)) and Node.js 22 or
later. The build exports the console (`web/hub`) and embeds it in the
binary, which is why it needs Node.

```bash
git clone https://github.com/lamplitlabs/hive.git && cd hive
cargo build                                                  # the whole workspace, console included
cargo install --path crates/hive --root ~/.hive --locked     # a release build, as ~/.hive/bin/hive
```

The first build compiles the whole workspace and writes several gigabytes
under `target/`. `HIVE_HUB_SKIP_BUILD=1 cargo build` skips the console build
when you only work on the Rust side. [docs/development.md](docs/development.md)
has the crate map, the console's dev server and the rest.

## Check

The project's check is one script. Hive's own agents run the same checks
before their work is delivered. Run it before you open a pull request.

```bash
sh scripts/verify-check.sh
```

It runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
-- -D warnings`, the check that keeps `hive-hand` small
(`scripts/check-hand-deps.sh`) and every test of the workspace, and ends
with `verify-check: ok`. It takes a while. For a quick loop, run the part
you changed:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test -p hive-harness            # one crate
```

The two web packages and the Mac app have checks of their own:

```bash
cd web/hub && npm install && npm run check     # the console: typecheck, lint, build, test
cd web/site && npm install && npm run check    # the site: the same four steps
apps/macos/scripts/check.sh                    # the Mac app (needs Xcode or the command line tools)
```

## Commit messages

`area: one plain sentence`, then a blank line, then a body that says why.

```text
harness: inline images take GIF the same as uploads

ImageAttachment::decode refused image/gif with "Invalid images" although
an uploaded GIF is served and shown; an owner who pasted a GIF got an
error instead of an answer. decode now accepts image/gif (saved as .gif),
the refusal names it, and the hub's bad-shape test uses image/bmp.
```

The area is the part you changed: a crate without its `hive-` prefix
(`runner`, `hub`, `harness`), a module, or a word such as `docs`, `site` or
`release`. The sentence says what is true after the change. The body says
what was wrong or missing, with a number where you measured one.

## Pull requests

Changes arrive as pull requests against `main`. Fork the repository, make a
branch, and open the pull request from it.

Hive works on this repository too: its agents open pull requests here, and a
review agent reads those and may merge them. That agent only ever merges
branches Hive pushed itself. **Your pull request is reviewed and merged by a
person.** Do not name your branch `hive/...` or `evolve/...`; those names
belong to Hive's own runs.

A pull request is easy to accept when:

- it makes one change, and the title says which;
- a test fails without the change and passes with it;
- the check passes;
- the page in `docs/` that describes the behaviour is updated in the same
  pull request. The docs are the reference, and they say only what the code
  does.

Write docs and messages in plain words and short sentences. Leave out what
you cannot confirm.

## Ask first

Some paths belong to the owner. Hive's constitution keeps its own agents
away from them, and the same holds for contributors: open an issue and get
a yes before you change

- `.github/` (the workflows, the code owners, the templates);
- `docs/decisions/` (the decision records: propose a decision in an issue,
  and the owner writes or accepts the record);
- the evolution records: the constitution, the evaluators and the stop file
  (`docs/evolution/` where a repository keeps them as files).

`.github/CODEOWNERS` also asks for the owner's review on the store's schema
(`crates/hive-store/`) and the console (`web/hub/`). A new dependency or a
major upgrade of one is a decision too, so ask before you add it.

## Bugs, features, questions

- **A bug:** open an issue with the *Bug report* form. Say what you did,
  what happened, what you expected, and what `hive version` prints.
- **A feature:** open an issue with the *Feature request* form. Start from
  the problem, not from the solution.
- **A question:** read [the docs](https://hivefleet.dev/docs/) and
  [Questions people ask](docs/faq.md) first, then open an issue with the
  *Question* form.
- **A vulnerability:** do not open an issue. [SECURITY.md](SECURITY.md)
  says how to report it privately.

Everyone who takes part is expected to follow the
[code of conduct](CODE_OF_CONDUCT.md).

## Licence

By contributing you agree that your contribution is released under the
[MIT licence](LICENSE) of this repository.
