# Snowflake table lifecycle attempt 9A8D9D3D

Prepared 12 November 2026. **Not executed; fresh explicit approval is required.**
The current connection uses external-browser SSO. This is one helper/Studio
qualification attempt on frozen source `103cb45`; it does not establish normal
app acceptance or qualify the newer relation-definition feature in `e218aef`.

## Exact identity and window

- Execution envelope SHA-256: `44fc7e5cfea8fd192342b208b3578fb54784e45db1118dd0c75adedac3b26f83`.
- Plan SHA-256: `bab8bb3d6c53b7024f85950f0fcb65a38bd813c787746c2f9b097dcbd3da22a2`.
- RUN: `9A8D9D3D1E7B43A3BE43389D902A8B0D`.
- Admission window: **12 November 2026, 05:52:00–06:22:00 UTC**
  (**12:52:00–13:22:00 ICT**, UTC+7).
- Sole write/data-read target:
  `SANDBOX.BILLIE_TESTING.JOINERY_QUAL_9A8D9D3D1E7B43A3BE43389D902A8B0D`.
- Copied executable SHA-256:
  `f58aa8e648e1a900aa8e6c135da0b88ad1563b3ee183d14c9c365687360428fc`.
- Runtime manifest SHA-256:
  `74e1dfac7b3346d163fad04879dc7f558c73111d2eac6230a904a98e34621f1f`.

The account, username, role and warehouse are preserved from the current
repository-local non-secret connection context; only the disposable database/
schema context is selected. No password, token or existing app/keychain profile
is read or stored by this preparation.

## Account effects and metadata disclosure

The [full lifecycle specification](snowflake-existing-schema-qualification.md)
and [physical SQL ledger](snowflake-qualification-sql-ledger.md) define the fixed
32-phase operation. Success creates the one new table with nullable ID
NUMBER(38,0), NOTE VARCHAR(128), ACTIVE BOOLEAN; adds nullable EXTRA VARCHAR(32);
verifies an INSERT is rolled back; verifies a second INSERT is committed; updates
and deletes that synthetic row; then drops only that generated table with
DROP RESTRICT. There are three DDL statements, four DML statements, two BEGINs,
one ROLLBACK and one COMMIT. The six data readbacks use LIMIT 2 as a sentinel;
only the new table is read, with at most one expected row. No CREATE DATABASE,
CREATE SCHEMA, OR REPLACE, CASCADE, existing-table data read or existing-object
write is included.

The read scope is broader than that table: startup session/version queries,
repeated session/settings observations, a census of visible SANDBOX schemas,
seven per-schema relation-inventory passes, additional BILLIE_TESTING inventories
and exact-target/column/descriptor/ownership checks. **Schema census SQL uses
LIMIT 1001; execution proceeds only if the accepted census has at most 32
schemas.** It does not promise to observe only 32 names before rejecting a larger
census. Relation inventories disclose existing object names and traits, with
10,000 aggregate relation / 2 MiB catalog bounds. Column inspection is confined
to the generated target, with 1,000-column / 1 MiB bounds.

The application SQL ceiling is **386 + 7×S**, at most **610** for 32 accepted
schemas. SDK authentication, result retrieval, cancellation and logout traffic
are additional requests outside that application-SQL count. This authorizes one
application attempt with external-browser SSO and no application retry; the SDK
may retransmit requests internally. Warehouse resume, warehouse/cloud
services and storage charges are possible. This is not a monetary cap. Local
120-second authentication, 30-second admission and 15-minute workflow deadlines
do not guarantee that submitted remote work stops. No additional cleanup SQL is
authorized if an operation fails or its outcome is unknown; the table may remain.

## Local effects and execution boundary

Execution would validate or create the fixed OS-account namespace
`Library/Application Support/Joinery/private-qualification/table-lifecycle-consumption-v1`
under the macOS effective user's physical home, outside this repository. It would
write permanent one-use RUN/digest markers there and private attempt evidence
under the reviewed repository-local `attempts` directory. Unsafe existing nodes
fail without repair. This does not adopt an existing Joinery app profile or
keychain. Failed or interrupted attempts retain their markers and evidence.

A private approval file would be created only after explicit approval for this
exact digest, window, account effects, local marker scope and possible billing.
The reviewed execute route then runs once, retaining original outcomes, shutdown
and terminal evidence. An expired window requires a new reviewed Plan and new
approval; neither earlier approvals nor elapsed time authorize execution.

## Verification and retained artifacts

Offline emit and a separate verify process both exit **0**, with
`accountActivity:false`. Independent comparison recomputes both digests, verifies
the embedded Plan and finds the old/fresh Plans identical after RUN substitution.
Every other profile/policy/source/runtime/executable/compiler pin is unchanged;
only RUN-derived Plan bytes/digest and the window differ. No account, helper,
SSO, native marker-store access or approval-file creation has occurred.

Private artifacts remain under
`.cache/snowflake-table-review-bbace18d50ca49919a73b3eb796bfeae/plans/plan-9A8D9D3D1E7B43A3BE43389D902A8B0D/`:
`plan.json`, `execution-envelope.json`, `offline-emission.json`,
`offline-verification.json` and `review-window.json`. The Plan retains legacy
`limits.*_missing` labels and does not inline all startup SQL; it must be reviewed
with the frozen helper SQL contract and Rust physical ledger. The
[offline command guide](snowflake-table-offline-command.md) documents the exact
execute arguments and receipt semantics. No generated or private artifact is
committed with this review document.
