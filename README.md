# Scrawny

An ELCI-group change-decomposition and anti-hunking tool. Scrawny analyses a
diff or a repository's uncommitted changes and reports how hard the change
is to review — not how many lines it touches, but how many independent
concerns it mixes together.

> Scrawny does not make code smaller. Scrawny makes change smaller.

## Status: Phase I — Observatory

Deterministic, offline, read-only analysis. No git mutation of any kind.

Implemented:

- Git diff acquisition (working tree, staged, HEAD, commit ranges) via a
  `Repository` trait (`scrawny-git`), backed today by the `git` CLI.
- Unified-diff parsing into structured `Change` records (`scrawny-core`).
- Heuristic concern classification — behavioural / structural / testing /
  documentation / mechanical / configuration / dependency / unknown — each
  with an explicit confidence, never a false certainty (`scrawny-analysis`).
- Review-load scoring and cohesion estimation, both multi-dimensional (never
  `lines_changed` alone).
- `analyse`, `explain`, `check`, `diff`, and a `suggest` stub that is honest
  about not yet proposing decompositions (that needs Phase II's dependency
  graph).
- `.scrawny.toml` policy configuration with explicit, named exceptions.
- `text` and `json`/`jsonl` output, versioned schema.

Not yet implemented (later phases per the project directive):

- AST-aware symbol/dependency extraction and semantic clustering (Phase II).
- Decomposition proposals and transactional commit splitting (Phase III).
- ELCI tool integration contracts — Ferret, Kaptaind, Padagonia, UNI (Phase IV).
- Automatic splitting (`scrawny split --apply`) (Phase V).

## Usage

```bash
scrawny                       # analyse uncommitted changes (working tree)
scrawny analyse --staged      # analyse only staged changes
scrawny explain               # analyse plus per-concern evidence
scrawny check                 # CI pass/fail against .scrawny.toml policy
scrawny suggest                # explains why decomposition isn't available yet
git diff | scrawny diff --stdin
scrawny --format json analyse
```

## Building

```bash
cargo build --workspace
cargo test --workspace
```

## Configuration

`.scrawny.toml` at the repository root:

```toml
[policy]
max_review_load = 70
min_cohesion = 0.55
max_concerns = 4

[policy.exceptions]
documentation = true
generated = true

generated = ["**/Cargo.lock", "**/*.generated.*"]
ignore = []
```
