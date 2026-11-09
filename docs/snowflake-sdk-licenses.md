# Snowflake SDK and runtime license evidence

Audit date: 2026-11-09. This is a bounded source and package-inventory record
for the scratch `snowflake-sdk` prototype, not legal advice, legal clearance,
or a final packaged bill of materials. No repository integration or release
artifact was audited.

## Pinned components

| Component | Version and evidence | License evidence | Status |
|---|---|---|---|
| Snowflake Node.js driver | `snowflake-sdk` 3.4.1; [npm package](https://www.npmjs.com/package/snowflake-sdk/v/3.4.1), [upstream tag](https://github.com/snowflakedb/snowflake-connector-nodejs/tree/v3.4.1), [tag manifest](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/package.json) | Apache-2.0; exact tag includes [`LICENSE.txt`](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/LICENSE.txt). Registry tarball SRI: `sha512-L4fvZJQdLO7Et7ul1fncfSQRrfs/32pl+sJiintBhk8xYiBpyUfgklAsKi9KCF8eUW3aItcyn1YP9dx8cMyJ9w==`. | Package version declares Node >=20. Prototype uses Node 24.22.2. This does not establish suitability of a final package. |
| Node.js runtime | 24.22.2, [official release](https://github.com/nodejs/nodejs.org/blob/main/apps/site/pages/en/blog/release/v24.22.2.md) dated 2026-10-28 | Release-tag license and bundled third-party license material copied and hashed in the audit (SHA-256 `5888dbb9a1d2b18f2c3e6c5f6af1b39de658372b402a0577b002777f14c62ace`). | Runtime binary was not audited as a packaged artifact. Include matching runtime notices and inspect the actual final bundle. |

The prototype SDK source inspection found statement retry middleware enabled
in 3.4.1; this is a behavior/qualification issue, not a licensing restriction.
See [driver design](snowflake-driver-design.md) for retry and outcome limits.

## Locked dependency inventory

The exact prototype lock contains 183 packages: 139 MIT, 38 Apache-2.0, 3 ISC,
1 Unlicense, 1 BSD-3-Clause, and 1 0BSD. The lock-derived registry tarball
SHA-512 integrity values were recomputed and matched. Eleven packages have no
root LICENSE/NOTICE/COPYING file in the tarball. They are all marked
production, non-optional, and non-peer in that lock; this does not prove which
code paths load at runtime or justify excluding them as dead code.

| Package | Declared license | Exact-source notice finding |
|---|---|---|
| `@aws-sdk/credential-provider-http@3.972.84` | Apache-2.0 | Matching package version and [monorepo LICENSE](https://raw.githubusercontent.com/aws/aws-sdk-js-v3/v3.1164.0/LICENSE) at AWS SDK JS v3 tag `v3.1164.0`. |
| `@aws-sdk/credential-provider-login@3.972.89` | Apache-2.0 | Matching package version and [monorepo LICENSE](https://raw.githubusercontent.com/aws/aws-sdk-js-v3/v3.1164.0/LICENSE) at the same AWS tag. |
| `@aws-sdk/nested-clients@3.998.1` | Apache-2.0 | Matching package version and [monorepo LICENSE](https://raw.githubusercontent.com/aws/aws-sdk-js-v3/v3.1164.0/LICENSE) at the same AWS tag. |
| `@nodable/entities@3.1.0` | MIT | Exact npm `gitHead` commit `ac48e7ea591da372be023a481875c747535812b3` has a [root LICENSE](https://raw.githubusercontent.com/nodable/val-parsers/ac48e7ea591da372be023a481875c747535812b3/LICENSE). |
| `agent-base@6.0.2` | MIT | Complete MIT notice in the README from the verified exact npm tarball; [corresponding upstream README](https://raw.githubusercontent.com/TooTallNate/node-agent-base/c4b8ea2e1a11bae023bb09b708050a50418204e9/README.md). |
| `asn1.js-rfc2560@5.0.1` | MIT | Full project MIT notice is available in [upstream README](https://raw.githubusercontent.com/indutny/asn1.js/v5.0.1/README.md) and [LICENSE](https://raw.githubusercontent.com/indutny/asn1.js/v5.0.1/LICENSE); exact npm source mapping remains unresolved. The tarball has no `gitHead`; npm code differs from repository tag `v5.0.1` ([tag source](https://raw.githubusercontent.com/indutny/asn1.js/v5.0.1/rfc/2560/index.js)): `responseExtensions` uses `explicit(1)` in npm versus `explicit(0)` at the tag. |
| `asn1.js-rfc5280@3.0.0` | MIT | Upstream [tag `v5.0.1`](https://raw.githubusercontent.com/indutny/asn1.js/v5.0.1/rfc/5280/package.json) contains this exact subpackage version and project [LICENSE](https://raw.githubusercontent.com/indutny/asn1.js/v5.0.1/LICENSE). |
| `data-uri-to-buffer@4.0.1` | MIT | Complete MIT notice in README from the verified exact npm tarball; [corresponding upstream README](https://raw.githubusercontent.com/TooTallNate/node-data-uri-to-buffer/85cd8c854aefbf1bb636789d80364cfac8ea1583/README.md). |
| `generic-pool@3.9.0` | MIT | Complete MIT notice in README from the verified exact npm tarball; [tag README](https://raw.githubusercontent.com/coopernurse/node-pool/v3.9.0/README.md). |
| `https-proxy-agent@5.0.1` | MIT | Complete MIT notice in README from the verified exact npm tarball; [corresponding upstream README](https://raw.githubusercontent.com/TooTallNate/node-https-proxy-agent/d0d80cc0482f20495aa8595f802e1a9f3b1b3409/README.md). |
| `proxy-agent-negotiate@1.1.0` | MIT | Package metadata declares MIT, but no notice text was found in the exact tarball or checked [upstream tag](https://github.com/TooTallNate/proxy-agents/tree/proxy-agent-negotiate%401.1.0) materials. This is an evidence gap, not evidence of an incompatible license. |

For AWS and Nodable, the referenced upstream license text is available but is
not present as a separate root file in the package tarballs. For four MIT
packages, the full notice appears in their exact package README and was
preserved verbatim. The asn1.js project notice is available, but the
RFC2560 npm source-to-tag mismatch remains unresolved. For
`proxy-agent-negotiate`, do not invent a notice from a sibling package or treat
the SPDX declaration as the notice text.

## Audit artifacts and redistribution gates

The reviewable audit directory is
`/tmp/joinery-snowflake-license-audit/`. Its `README.md` records exact upstream
URLs, registry tarball SRI and local SHA-256 values, source commits/tags, and
hashes for copied original notice text under `upstream-texts/`. The original
license texts were not modified. The dependency inventory is from
`/tmp/joinery-snowflake-driver-prototype/LICENSE-INVENTORY.json`. These are
local scratch artifacts, not yet repository package notices.

Before redistributing an application bundle:

1. Include the applicable original SDK, dependency, and Node runtime license
   and notice text. Resolve the `proxy-agent-negotiate` notice evidence gap
   from an authoritative source; preserve the RFC2560 source-mapping caveat.
2. Re-run inventory against the exact final lock and packaged files, including
   platform-specific Node binaries and any native/runtime components. Confirm
   that required notices are actually shipped with the product.
3. Review the final dependency tree and distribution obligations. This audit
   does not establish legal clearance or whether a package is reachable in a
   particular application path. No npm lifecycle scripts were run for the
   scratch audit.

Until those steps are complete, neither SDK selection nor these findings
constitute approval to redistribute a production package.
