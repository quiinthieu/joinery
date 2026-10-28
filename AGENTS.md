# Joinery working agreements

These instructions travel with the repository. Keep essential project rules here;
do not depend on machine-specific agent files or tools being installed.

- Preserve unrelated edits, secrets, and running processes. Never automatically
  stash, reset, clean, or discard work. Stage only task-owned changes.
- Fetch `origin/main` without pulling, then create a task branch from it. Never
  commit directly on `main`. Use small Conventional Commits that pass the relevant
  checks. Do not amend, rebase, force-push, push, or merge without authorization.
- This is the personal `quiinthieu/joinery` repository. For the owner's work, run
  `pnpm setup:git` once per clone/worktree, then `pnpm check:git` before each commit.
  This sets only the checkout's email; keep global work identities and hooks.
  Before pushing, run `pnpm check:git --head` to check the actual commit author.
  The scripts also run directly with Node when pnpm is unavailable.
- `.github/commit-identity.json` lists approved author emails. Other contributors
  must use their own identity and arrange verified GitHub/Vercel access before
  adding their address through review. Never impersonate the repository owner.
- Read `README.md` for setup and verification. Run `pnpm validate` for application
  changes, plus the relevant fixture/browser/native tests for changed behavior.
  For instructions/tooling-only changes, run the relevant script tests and checks.
  Report pre-existing failures and blocked checks accurately; do not bypass hooks.
- Use disposable local database fixtures. Production queries and external engine
  qualification need explicit scope. Keep secrets out of source, logs, and commits.
- The private Joinery project hub linked in README is the canonical plan/status.
  Update it after product implementation, merge, and release milestones when Sites access
  is available; report unavailable access. Distinguish tested, merged, published,
  installed, and externally qualified states. Keep the Site owner-private.
- Ship only when authorized. Verify CI, release assets and the installed version
  before claiming delivery. Signing/notarization and external qualification gates
  remain open until demonstrated; see `docs/IMPLEMENTATION_STATUS.md`.
