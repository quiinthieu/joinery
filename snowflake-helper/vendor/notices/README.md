# Supplemental dependency notices

These four files preserve original upstream notice bytes for seven dependencies
whose exact npm packages omit complete license text. They supplement the package
files; they do not replace or modify them. `provenance.json` binds each record to
the helper lockfile, package name/version, public registry URL and tarball SRI,
installed evidence hashes, immutable source commit and original artifact hashes.
The four complete MIT READMEs already shipped by their packages have separate
hash-verified records and need no supplemental copy.

| Original file                 | Packages                      | Evidence scope                                                                                                                                                                                                                                                                                                   |
| ----------------------------- | ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `aws-sdk-LICENSE`             | Three listed AWS SDK packages | Root Apache-2.0 license at the commit resolved from `v3.1164.0`; same-commit package manifests match the npm names and versions. Compiled npm output is not claimed byte-identical to source.                                                                                                                    |
| `entities-LICENSE`            | `@nodable/entities@3.1.0`     | Root MIT license at the exact npm metadata `gitHead`. The upstream subpackage manifest still says 3.0.0; a matching source version or build is not claimed.                                                                                                                                                      |
| `asn1-LICENSE`                | Both listed RFC packages      | Same original MIT bytes at both recorded commits. RFC2560's two published files match its subpackage release commit. RFC5280's published source matches the immutable main-package release commit, whose subpackage manifest has the correct name/version. No signed publisher-to-commit attestation is claimed. |
| `proxy-agents-shared-LICENSE` | `proxy-agent-negotiate@1.1.0` | Complete original MIT notice from a sibling package at the release commit. **Applicability to negotiate remains unresolved.** The notice is evidence, and this package remains classified incomplete.                                                                                                            |

Preparation reads only these repository files; it performs no notice downloads.
It validates their hashes, source records, exact lock bindings and installed
evidence before promoting a stage. Copies live under `notices/supplements/`,
alongside the exact provenance manifest. Original package files remain under
`notices/dependencies/`, including READMEs that contain only a license label.
Such a README alone is classified incomplete. The full official Node release
`LICENSE` remains under `notices/node/`.

Coverage means preserved notice evidence, not legal clearance, verification of
publisher signatures, or authentication of every dependency file. In particular,
the proxy package remains in `packagesWithIncompleteNotice` even though its
sibling notice is copied. Packaging must preserve this unresolved status.

To review an update, verify the registry tarball against the new lock SRI, inspect
its original notice inventory, retrieve the recorded immutable upstream artifacts
and recompute their SHA-256 hashes. Check the exact source mapping stated in each
record rather than assuming a monorepo version tag proves build equivalence.
Update notice bytes only from the original upstream text; keep additions and
qualifications in this README or provenance. Run the preparation script tests.
An updated local stage still requires separate qualification.
