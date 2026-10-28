import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const policy = JSON.parse(
  readFileSync(new URL("../.github/commit-identity.json", import.meta.url), "utf8"),
);
const git = (...args) => execFileSync("git", ["-C", root, ...args], { encoding: "utf8" }).trim();

function requireEmail(email, source) {
  if (!policy.allowedAuthorEmails.includes(email)) {
    throw new Error(
      `${source} uses ${email}; approved emails: ${policy.allowedAuthorEmails.join(", ")}. ` +
        "Run pnpm setup:git in this checkout and remove conflicting GIT_AUTHOR_EMAIL / GIT_COMMITTER_EMAIL overrides. " +
        "Changing Git config does not change existing commits. See README.md for recovery steps.",
    );
  }
}

try {
  const mode = process.argv.slice(2);
  if (mode.length > 1 || (mode.length === 1 && !["--setup", "--head"].includes(mode[0]))) {
    throw new Error("Usage: node scripts/git-identity.mjs [--setup | --head]");
  }
  if (mode[0] === "--head") {
    requireEmail(git("show", "-s", "--format=%ae", "HEAD"), "HEAD author");
    console.log("HEAD author matches the approved commit identity.");
  } else {
    if (mode[0] === "--setup") {
      requireEmail(policy.setupEmail, "Setup policy");
      git("config", "--local", "user.email", policy.setupEmail);
    }
    for (const role of ["AUTHOR", "COMMITTER"]) {
      const ident = git("var", `GIT_${role}_IDENT`);
      const email = ident.match(/<([^<>]+)> \d+ [+-]\d{4}$/)?.[1];
      if (!email) throw new Error(`Could not read the effective Git ${role.toLowerCase()} email.`);
      requireEmail(email, `Effective ${role.toLowerCase()}`);
    }
    console.log("Effective Git author and committer match the approved commit identity.");
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
