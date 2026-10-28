import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const expected = JSON.parse(
  readFileSync(new URL("../.github/commit-identity.json", import.meta.url), "utf8"),
).setupEmail;

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "joinery-git-identity-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, "scripts"));
  mkdirSync(join(root, ".github"));
  copyFileSync(
    new URL("./git-identity.mjs", import.meta.url),
    join(root, "scripts/git-identity.mjs"),
  );
  copyFileSync(
    new URL("../.github/commit-identity.json", import.meta.url),
    join(root, ".github/commit-identity.json"),
  );
  const env = {
    ...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_"))),
    GIT_CONFIG_GLOBAL: join(root, "global.gitconfig"),
    GIT_CONFIG_NOSYSTEM: "1",
  };
  function run(command, args, overrides = {}) {
    return spawnSync(command, args, {
      cwd: root,
      env: { ...env, ...overrides },
      encoding: "utf8",
    });
  }
  function git(...args) {
    const result = run("git", args);
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  }
  git("init", "-q");
  git("config", "--global", "user.name", "Fixture owner");
  git("config", "--global", "user.email", "work@example.invalid");
  git("config", "--local", "core.hooksPath", "custom-hooks");
  const check = (args = [], overrides = {}) =>
    run(process.execPath, ["scripts/git-identity.mjs", ...args], overrides);
  return { git, check, root };
}

test("rejects an inherited work identity, then configures only the local email", (t) => {
  const { git, check, root } = fixture(t);
  const globalBefore = readFileSync(join(root, "global.gitconfig"), "utf8");
  assert.equal(check().status, 1);
  assert.equal(check(["--setup"]).status, 0);
  assert.equal(check().status, 0);
  assert.equal(git("config", "--local", "user.email"), expected);
  assert.equal(git("config", "user.name"), "Fixture owner");
  assert.equal(git("config", "core.hooksPath"), "custom-hooks");
  assert.equal(readFileSync(join(root, "global.gitconfig"), "utf8"), globalBefore);
});

for (const variable of ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"]) {
  test(`rejects a conflicting ${variable} even after setup`, (t) => {
    const { check } = fixture(t);
    const result = check(["--setup"], { [variable]: "override@example.invalid" });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /override@example\.invalid/);
  });
}

test("CI rejects an incorrectly authored HEAD even after fixing local config", (t) => {
  const { git, check } = fixture(t);
  git("commit", "--allow-empty", "-qm", "Fixture commit");
  assert.equal(check(["--setup"]).status, 0);
  const result = check(["--head"]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /HEAD author uses work@example\.invalid/);
});

test("CI accepts an approved author with a GitHub automation committer", (t) => {
  const { git, check } = fixture(t);
  git(
    "-c",
    "user.email=noreply@github.com",
    "commit",
    "--allow-empty",
    "-qm",
    "Fixture commit",
    "--author",
    `Fixture owner <${expected}>`,
  );
  assert.equal(git("show", "-s", "--format=%ce", "HEAD"), "noreply@github.com");
  assert.equal(check().status, 1);
  assert.equal(check(["--head"]).status, 0);
});

test("rejects unknown arguments before changing configuration", (t) => {
  const { git, check } = fixture(t);
  assert.equal(check(["--setup", "--head"]).status, 1);
  assert.equal(git("config", "user.email"), "work@example.invalid");
});
