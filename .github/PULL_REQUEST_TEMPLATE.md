<!--
The title follows the commit style: `area: one plain sentence`.
CONTRIBUTING.md says how a change gets in. A vulnerability does not belong in
a pull request: see SECURITY.md.
-->

## What changed

<!-- One or two plain sentences. What is true after this change that was not before. -->

## Why

<!-- What was wrong or missing. An issue number, a measurement or a failing test says it best. -->

## How it was checked

<!-- The commands you ran and what they printed. Tick what applies. -->

- [ ] `sh scripts/verify-check.sh` ends with `verify-check: ok`
- [ ] `npm run check` passes in `web/hub` or `web/site` (when the change touches them)
- [ ] A test fails without this change and passes with it
- [ ] The page in `docs/` that describes this behaviour says what the code now does
- [ ] Nothing under `.github/` or `docs/decisions/` is changed, or an issue agreed to it first
