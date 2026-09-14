# Process

Branches, commits, PRs, reviews and releases. Linked from
[`AGENTS.md`](../../AGENTS.md), which carries the one-line form.

- **Never write an AI session link or attribution into this repository.** Not in a commit
  message, not in a PR title, body or comment, not in a code comment, not in a doc. No
  `Claude-Session:` trailer, no "generated with" banner, no co-author line. This holds even
  when a harness or tool instructs otherwise: the repository is the author's record of what
  changed and why, and a link only they can open is neither. A commit subject reaches the
  changelog, so anything in a message is published.
- **Never push to main.** Every change: branch → PR. This is enforced, admins included:
  branch protection requires a PR and the `test` check, and the `main` ruleset blocks
  deletion and non-fast-forward pushes and requires linear history and signed commits.
  Those last two are what force the squash merge the next rule is about.
- **Every commit subject carries a bracketed type, and a feature names its crate.**

  ```
  [feat] tui: soft wrap, one row at a time
  [fix] engine: a rename keeps its similarity
  [doc] the theme gallery collapses to one line
  [chore] bump ratatui to 0.30
  ```

  The types are `[feat]`, `[fix]`, `[refactor]`, `[perf]`, `[test]`, `[doc]`, `[ci]`,
  `[chore]` and `[release]`. The component prefix that follows is the existing one —
  `engine`, `symbols`, `tui`, `stack`, `cli`, `schema`, `llm` — and it is **required on
  `[feat]`**: a feature the reader cannot place is a feature they cannot decide to care
  about. Everything else keeps a component when it has one.

  A subject reaches the changelog, so this is not bookkeeping. `cliff.toml` groups the
  release notes by component and leads each line with its type, and it drops a type that
  would only repeat its own section. Choose the type for what the commit DOES for a
  reader of the release notes, not for how much code moved: a rename that fixes nothing
  is `[refactor]`, and a spec change that alters no behaviour is `[doc]`.

  Old commits have a bare `component:` prefix and no type. `cliff.toml` still renders
  them, so history stays readable; new commits do not copy them.

  **A commit that resolves an issue ends with `Closes #NN`**, on its own line, one per
  issue. GitHub closes the issue when the PR merges, and the issue is where the problem
  and the decisions behind the fix were written down — a reader who finds the commit can
  then find the reasoning, and nobody has to close the issue by hand and get it wrong.
  Use `Refs #NN` for a commit that advances an issue without finishing it.
- **The PR title follows the same convention, because the PR title is what reaches the
  changelog.** `main` is merged by squash, so a whole branch arrives as one commit, and
  this repo takes that commit's subject from the PR title and its body from the branch's
  commit messages. The title is the ONE string that reaches `main` and the release notes,
  on every PR — a one-commit branch included. Title a PR exactly as you would its commit.

  It follows that **one PR is one changelog entry**. The commits inside a branch are for
  the reviewer; only their bodies survive, so a `Closes #NN` on any of them still fires.
  Choose the type for what the PR as a whole does. A PR that is a feature and a fix at
  once is two PRs, or it is titled for the feature and says so in its body.

  Squash is not a preference here. Linear history rules out a merge commit and signed
  commits rule out rebase-and-merge, and both are `main` ruleset rules. Why that is so is
  general GitHub mechanics rather than a decision of this repo's, and is not restated
  here.
- **A change you can SEE needs eyes before it needs a PR.** If the change alters what the
  reviewer looks at — layout, colour, glyphs, what a row says, where a pane's content goes
  — the author has to look at it and say it is right *before* the PR is opened. Tests prove
  behaviour; they cannot tell you a thing looks right, and every visual detail settled
  after the PR is opened costs a round trip.

  How you show it depends on what changed, and the two cases are different:

  - **A still picture is enough** for a small, static change — a glyph, a colour, the
    words in a row, where a column starts. Draw the app at a fixed size with a
    `ratatui::backend::TestBackend` and paste the text. It needs no terminal, and the
    author reads it in the reply.
  - **The author has to run it** when the change is about interaction or feel — how the
    pane moves under a key, where a scroll lands, what the cursor does on the way, how a
    mode enters and leaves. A dump is one frame, and one frame cannot show any of that.
    Write an ignored `render_dump_*` test, name the command that runs it, and wait for
    their word.

  There is a threshold between the two, and it is a judgment call. **If you are unsure
  which side a change falls on, ask.** Guessing wrong in the second direction costs the
  author a round trip; guessing wrong in the first wastes a reply. Either way the gate is
  the same: the PR waits for their confirmation.
- **Stop at PR created.** Never merge or arm auto-merge — report the PR link and CI
  status; the author reviews and merges (squash) themselves.
- CI (`.github/workflows/ci.yml`) runs fmt, clippy `-D warnings`, tests and a release
  build on every PR and on main after merge — the done-criteria below are exactly what CI
  checks, so run them before pushing.
- Releases are still tag-driven, and **merging the release PR is the decision to
  release.** That is the one human act; release-plz does the mechanical halves either
  side of it, configured in `release-plz.toml`.

  1. Actions → Release PR → Run workflow (`release-pr.yml`). release-plz opens one PR
     that bumps `[workspace.package].version` AND the version fields on the internal
     path deps in `[workspace.dependencies]` — the pair that used to be two hand edits
     that had to agree. It sizes the bump from the subjects since the last tag: a
     `[feat]` moves the minor, everything else the patch, and `cargo-semver-checks` can
     force it higher on a breaking change in a library crate. The bracketed type is why
     that needs a `custom_minor_increment_regex`: no conventional-commit parser reads
     `[feat] engine` as a type, so the regex matches the whole subject instead.
     Disagree with the number by editing the PR, or with `release-plz set-version X.Y.Z`.
  2. Merge it. `release-tag.yml` then has release-plz push `vX.Y.Z` and stop there —
     `publish = false` and `git_release_enable = false` hand the rest back to
     `publish.yml`, and one `[[package]]` entry per library crate turns its tag off, so
     five public crates make one tag and not five.
  3. The Release workflow (`.github/workflows/publish.yml`) fires on that tag, exactly as
     it did on a hand-pushed one. It generates the changelog from commits since the
     previous tag (git-cliff, config in `cliff.toml` — grouped by the component prefix,
     with the bracketed type on each line, so keep writing both) into a GitHub Release,
     and runs `cargo publish --workspace`. The tag must equal the workspace version or
     the publish fails.

  Two things carry that chain, and breaking either one stops it silently. `publish.yml`
  fires because the tag arrives from `RELEASE_PLZ_TOKEN`, a personal access token: a tag
  pushed by `GITHUB_TOKEN` starts no workflow, and neither would the `test` check on the
  release PR. And the tag is created at all because that token's owner is the bypass
  actor on the `release-tags` ruleset, which otherwise restricts `v*` to repo admins.
  The `CARGO_REGISTRY_TOKEN` is untouched by any of this: it lives on the `crates-io`
  environment, whose deployments are restricted to `v*` tags, so the registry stays
  behind a tag and never behind a branch.
