<!--
- Have you run `just check` locally? `just ci` if pipeline-touching?
- Is there a regression test (or new test for new behaviour)?
- Did you update `docs/CHANGELOG.md` under `[Unreleased]` for any
  user-visible change?
-->

## What this changes

<!-- One paragraph: what behaviour does the user see after this PR
that they didn't see before? -->

## Why

<!-- Link an issue, an upstream report, or describe the motivation. -->

## How

<!-- Sketch of the approach. If this touches anything called out in
the load-bearing-invariants section of AGENTS.md (firewall,
slot queue tuning, alpha pre-key, etc.), explain why the change
preserves the invariant. -->

## Verification

- [ ] `just check` passes locally
- [ ] `just ci` passes locally if this touches the pipeline or packaging
- [ ] Added a test or regression test
- [ ] Updated `docs/CHANGELOG.md` under `[Unreleased]` for user-visible changes
