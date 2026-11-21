---
name: Task or slice
about: A planned piece of work from the build plan, or a feasibility gate
title: "slice N: short description"
labels: task
---

## Goal

What will be true when this is done, in one or two sentences.

## Context

Link the build plan slice, feasibility gate, ADR or research section this comes from.

## Scope

- [ ] Item to build
- [ ] Item to build

**Out of scope:** what this issue deliberately does not cover.

## Tests and evidence

- [ ] Test or measurement that proves it works
- [ ] Results saved in `docs/benchmarks/` when this is a gate

## Done when

- [ ] Tests pass locally and in CI
- [ ] `docs/architecture.md`, the build plan progress table and `CHANGELOG.md` updated
- [ ] ADR written if a decision was made
- [ ] Pull request reviewed and merged with `Closes #<this issue>`
