# Isolated query-tab sessions

M06-Q is a local implementation of bounded console/session ownership for
PostgreSQL and MySQL/MariaDB. Broader M06/E14/X04 remain PARTIAL. The private hub
remains canonical shipping status; this bounded local workflow is verified
and unreleased. External account/auth/platform qualification remains open.

## Workflow and ownership

Query tabs keep their existing shared session by default. New session opens the
existing sign-in dialog and binds its newly authenticated Rust session to that
tab. PostgreSQL/MySQL/MariaDB use this action; Snowflake's SQL API and Redis
retain their existing workflows. Each isolated tab owns one session. The existing
Rust limit of four sessions per saved connection still applies.

The connection uses the saved profile and its normal credential/TLS/SSH flow.
Credentials remain in that dialog or the OS store and are cleared from dialog
state after submission. No driver credentials or SQL session state are cloned.
The existing primary session remains responsible for the explorer/table browser.
Isolated query execution and completion use the tab's actual session ID.

Cancellation retires only the affected session. Its tab keeps a disconnected
binding and requires explicit sign-in; it never falls back to the shared socket
or replays SQL. A late prepared/reviewed plan is not executed if the tab's session
changed. Late close events compare exact IDs, preserving a newer connection.

Closing the tab releases results and disconnects its owned session. Close,
connection change, profile disconnect and native exit warn about possible open
transactions when an isolated session is connected. Profile disconnect closes
all its sessions. A late connection to a removed/rebound/busy tab is released.
Draft recovery persists SQL and profile identity only, never live bindings,
credentials, transactions or results.

## Architecture and limits

Rust already owns multiple session handles, execution lanes, cancellation,
approvals, tunnels and result budgets. This slice reuses connect/disconnect and
the existing typed IPC. It adds only transient querySession/querySessionClosed
fields to tabs in the existing Zustand store, plus querySessionForTab,
bindQuerySession and retireSession. No new backend execution or credential API
is introduced. CodeMirror, Glide and Arrow retention are unchanged.

There is no transaction-state detector, autocommit toolbar, savepoint UI,
named-session manager, multi-client attachment to an isolated session or
persistent live-session recovery. Shared-session behavior remains available.
Production accounts and additional isolated-session SSH/TLS/auth variants need
their own qualification. This is not complete DataGrip session parity.

## Acceptance and evidence

Sources: src/state/studio.ts; App.tsx runSql/executePrepared/health/close;
Workbench session controls and per-session completion epoch; ConnectDialog's
optional session handoff. Rust execution.rs connect/disconnect and its four-session
limit are reused unchanged.

- Eight new unit cases cover tab ownership, result release, exact retirement,
  stale/late responses, sibling isolation, credential handoff and draft safety.
- Three actual browser/database flows cover PostgreSQL 18, MySQL 8.4 and
  MariaDB 11.4: private versus shared state, a shared query while another session
  is running, owned cancellation, explicit reconnect, close cancellation and
  cleanup observed through Rust resource usage. PostgreSQL also delays a prepared
  response across rebinding and verifies no execution submission.
- The corrected pre-implementation acceptance baseline reproduced the missing
  isolated-session control. Initial result-selection helper errors were test
  setup issues, not application gaps.
- Final pnpm validate passes 71 frontend and 86 standard Rust tests, formatting,
  types, frontend build, Cargo check and strict Clippy.
- The final fixed-file full browser run passes all 26 workflows, including the
  grid keyboard assertion and the three new engine flows. Formatting also passes.
- Earlier full attempts each passed 25/26: a module-loading network interruption,
  then a repeated-click value-viewer issue in the existing keyboard helper.
  Retries were interrupted at host load above 100/126 and low free memory.
  After resources recovered, an unconditional Escape helper also failed by
  clearing a normal grid selection. The final helper dismisses only an open
  Glide overlay and verifies the selection before ArrowRight. The final suite
  passes without application changes for those failures.
- Desktop build, macOS bundle with strict ad-hoc signature, native smoke build
  and three hidden native runs/30 cleanup cycles pass. Native elapsed times:
  3662/3237/3423 ms. Build sourceRevision=f556518, sourceDirty=true because the
  final test-helper change preceded its commit. Native SHA-256:
  `a2d918af11f6910185f863ac9732c56de864b62bde02e5abbff47b562b5f953b`.
  Evidence: .cache/isolated-native-runs.json. This is local qualification, not
  clean-source release provenance or representative performance qualification.

Logs use .cache/isolated-*; fixtures are disposable. No production queries or
destructive runtime SQL are part of these tests.

## Official comparison

DataGrip 2026.2 documentation (reviewed 29 October 2026) supports shared or
separate console sessions and attaching consoles to existing sessions. Joinery's bounded slice currently supports shared or
tab-owned sessions, with explicit sign-in and ownership.

- [Query consoles](https://www.jetbrains.com/help/datagrip/query-consoles.html)
- [Manage sessions](https://www.jetbrains.com/help/datagrip/managing-connection-sessions.html)
