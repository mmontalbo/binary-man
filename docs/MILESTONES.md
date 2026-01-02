# Milestones

This document records the current milestone plan and status. It is the canonical
sequence for the project.

## M0 — Scaffold & Invariants (done)

Goal: Make the project reproducible and lock epistemic rules before any validation logic.

What landed:
- Nix flake + direnv dev environment (Linux-safe tooling).
- Rust CLI skeleton (claims, validate, regenerate).
- Schemas with explicit provenance and unknown handling.
- Binary identity hashing and env snapshots.
- Repo hygiene (fixtures/, out/, reports/, gitignore).
- Commit discipline tooling.

Invariants established:
- The binary is the source of truth.
- Docs/help are claims, not truth.
- Unknowns are first-class; no guessing.

## M1 — Surface Claim Ingestion (done)

Goal: Turn documentation into auditable, deterministic surface claims.

What landed:
- Conservative --help parser.
- Canonical option IDs (prefer long options).
- Separate claims for option existence and explicit argument arity (=ARG, [=ARG] only).
- Full audit fields (extractor, raw_excerpt, source).
- Golden snapshot test for ls --help.

Explicitly not done:
- No behavior semantics.
- No validation.
- No inference beyond explicit syntax.

## M2 — Surface Validation: Option Existence (done)

Goal: Validate that claimed options actually exist in the binary.

Scope:
- Validate only claim:option:*:exists.
- Execute the binary under controlled env.
- Classify each claim as confirmed/refuted/undetermined.
- Record evidence for every attempt.

Deliverable:
- ValidationReport tied to a concrete BinaryIdentity.

Deferred:
- Arity validation.
- Behavior validation.
- Man page generation.

## M2.5 — Surface Validation: Explicit Arity

Goal: Validate only what the docs explicitly claim about argument syntax.

Scope:
- Validate claim:option:*:arity where syntax is explicit.
- Required vs optional args only.
- Still no semantics.

Deliverable:
- Extended ValidationReport with arity results.

## M3 — Minimal Regeneration

Goal: Prove the pipeline can emit a truthful doc artifact.

Scope:
- Generate a minimal man page that includes:
  - Confirmed options
  - Refuted options (flagged)
  - Undetermined options (explicitly listed)
- Include binary hash/version header.
- Intentionally barebones.

## M4 — Coverage Accounting / “Comprehensiveness”

Goal: Make “comprehensive” measurable, not aspirational.

Deliverable:
- Coverage report:
  - % option existence confirmed
  - % arity confirmed
  - counts of undetermined claims
- Stop when:
  - surface completeness achieved
  - remaining gaps explicitly documented

## M5 — Selective Behavior Validation

Goal: Validate a small set of stable, high-signal behaviors.

Example (for ls):
- dotfiles hidden by default
- -a shows dotfiles
- -d lists directory itself
