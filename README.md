# Scrawny

An ELCI-group change-decomposition and anti-hunking tool. Scrawny analyses a
diff or a repository's uncommitted changes and reports how hard the change
is to review — not how many lines it touches, but how many independent
concerns it mixes together.

> Scrawny does not make code smaller. Scrawny makes change smaller.

## Status: Phase III — Git Workflow

Deterministic, offline, read-only analysis. No git mutation of any kind.

Implemented:

- Git diff acquisition (working tree, staged, HEAD, commit ranges) via a
  `Repository` trait (`scrawny-git`), backed today by the `git` CLI.
- Unified-diff parsing into structured `Change` records (`scrawny-core`).
- Heuristic concern classification — behavioural / structural / testing /
  documentation / mechanical / configuration / dependency / unknown — each
  with an explicit confidence, never a false certainty (`scrawny-analysis`).
- Review-load scoring, multi-dimensional (never `lines_changed` alone).
- Rust AST symbol extraction (`scrawny-parser`, via `syn`) and same-file
  symbol/dependency-graph clustering (`scrawny-cluster`) — real cohesion
  grounded in which functions a change actually touches and reference,
  not textual proximity or shared concern labels. Falls back to Phase I's
  concern-fraction cohesion proxy when no symbol data resolves (non-Rust
  diffs, or content that isn't available to read).
- Sub-hunk splitting: when a hunk (as git emits it, with its default 3-line
  context) spans more than one symbol, extraction splits it back into one
  `Change` per symbol so clustering isn't fooled by git's context merging
  two unrelated nearby edits into one hunk.
- `analyse`, `explain`, `check`, `diff`, and `suggest` — which now proposes a
  real decomposition (ranked clusters + evidence + an estimated review-load
  reduction) whenever clustering finds more than one independent unit of
  work, and says plainly when a change is already one coherent thing.
- `.scrawny.toml` policy configuration with explicit, named exceptions.
- `text` and `json`/`jsonl` output, versioned schema (`clusters` is real
  data now, not a reserved-empty placeholder).
- Wired into `kaptaind`'s pre-push advisory gate alongside Hybreed and
  Emulsify (`kaptaind` repo, `src/integration.rs`) — `check --stdin`/`--file`
  score an arbitrary supplied diff against a target repo's own
  `.scrawny.toml`, selected via `--path`, without needing to discover or
  walk a git repo of its own.
- `scrawny split` (`scrawny-split`, `scrawny-git`'s worktree module):
  constructs a real, validated series of commits from the detected
  clusters — full reconstructed file content per commit, never a guessed
  textual patch — entirely inside a disposable temporary `git worktree` on
  a throwaway branch. Before ever presenting a plan, it replays every
  commit's cumulative effect and checks the result against the real target
  content byte for byte; a mismatch is a hard error, not a plan. The
  branch and its commits are real, reachable from the main repo
  (`git log scrawny/split-<id>`) and persist after the scratch worktree is
  cleaned up — but the current branch and checkout are never touched, and
  nothing is merged, reset, or pushed automatically. `--apply`-style
  automation onto the real branch stays deliberately unimplemented
  (Phase V, per §15 of the project directive: "`--apply` must never be
  part of the default path").

Known limitation: clustering edges (same symbol, dependency reference) are
scoped to a single file by design — cross-file relationships would need real
import/type resolution to stay honestly evidenced. A change that's
genuinely coherent but spans many files with only architectural (not
textual) links between them will still score as lower-cohesion than it
"should", and `split` will propose more commits than strictly necessary for
such a change — this is Phase IV territory, not a bug to game around.
`split` also only reconstructs `Modified`/`Added` files at line granularity;
`Removed`/`Renamed`/`Copied` files are assigned to a cluster atomically
(whichever cluster owns their associated change), not decomposed further.

Not yet implemented (later phases per the project directive):

- `scrawny split --interactive` (review/adjust a plan before it's built).
- Cross-file/module-level relationship modelling; deeper ELCI tool
  integration contracts — Ferret, Padagonia, UNI (Phase IV).
- Automatic splitting onto the real branch (`scrawny split --apply`) (Phase V).

## Usage

```bash
scrawny                       # analyse uncommitted changes (working tree)
scrawny analyse --staged      # analyse only staged changes
scrawny explain               # analyse plus per-concern evidence
scrawny suggest                # real decomposition proposal, when there is one
scrawny split                  # build + validate real commits in a disposable worktree
scrawny split --discard-worktree   # same, but clean up the scratch directory after
scrawny check                 # CI pass/fail against .scrawny.toml policy
scrawny check --stdin --path <repo>   # check a supplied diff against <repo>'s policy
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
