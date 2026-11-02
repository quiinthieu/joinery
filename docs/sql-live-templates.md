# SQL live templates

Type a trigger and invoke SQL completion, or choose **Insert template** above
the editor. In Preferences, add a custom snippet globally or for PostgreSQL,
MySQL/MariaDB or Snowflake. Engine-specific snippets override global snippets;
custom snippets override built-ins with the same trigger.

Use `${1:editable text}`, `${2:next field}`, and `${0}` for the final cursor
position. Tab and Shift+Tab move between fields. Each expansion is one undo step.

| Variable | Value |
| --- | --- |
| `{{database}}` | Current database observed on this console, as one quoted SQL identifier |
| `{{schema}}` | Current schema observed on this console; current database for MySQL/MariaDB, as one quoted SQL identifier |
| `{{selection}}` | Selected editor text, inserted literally |

Database/schema expansion waits for the console context and reads its current
runtime state. PostgreSQL and Snowflake identifiers use double quotes;
MySQL/MariaDB use backticks. Embedded quote characters are doubled. Do not add
quotes around these variables or use them as SQL string values. Missing context,
a failed transaction, an unsupported/incomplete variable, or a changed editor or
console blocks insertion. Double braces are reserved for this vocabulary.

For example, a custom template can use:

```text
SELECT ${1:*}
FROM {{schema}}.${2:orders};${0}
```

The built-in `ctxsel` follows this pattern. To surround SQL, select a complete
query without its final semicolon and choose the built-in `wrap` template. Its
`{{selection}}` preserves the selected text, indentation and literal `${...}` or
`{{...}}` sequences; these are never expanded again. Template text outside the
selection follows the editor's normal snippet indentation.

Expansion only edits the document. Review the resulting SQL and explicitly run
it when ready. Closing the chooser, editing the document, changing the selection
or switching/refreshing the console while context is loading prevents a delayed
insertion. Expanded text is limited to 1 MiB UTF-8. Custom bodies retain the
existing 16 KiB storage limit and travel unchanged through metadata backup/restore.

These variables provide local editor convenience. SQL completion places the
built-in statement templates at the start of a statement, `join` and `ljoin`
in a top-level FROM or JOIN list, and `lim` after a top-level FROM. A custom
snippet may declare one of those sites. A missing declaration, including a
snippet saved before this field existed, stays available in every completion
site. A custom snippet that reuses a built-in trigger keeps that trigger's
place. **Insert template** still lists every template. A parenthesized query
after `FROM`, `JOIN`, `AS`, `IN`, `EXISTS`, `SELECT`, a comparison, or a comma
uses that same placement. A function name does not open one. A statement after
`EXPLAIN`, including `ANALYZE`, `VERBOSE`, a closed option list, or
`FORMAT=JSON`, uses that same placement. A parenthesized statement there does
too: `EXPLAIN ANALYZE (SELECT id FROM ` offers the JOIN and LIMIT templates,
while `EXPLAIN (ANALYZE)` stays an option list. New metadata includes the site, so an
older build that rejects unknown snippet fields cannot load a backup from this
build. Arbitrary expressions/macros and broader DataGrip template parity remain
open.
Snowflake quoting has local unit coverage; external Snowflake qualification
remains separate.
