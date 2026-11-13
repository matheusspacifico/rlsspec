# Output formats and exit codes

`rlsspec test`, `lint` and `cover` print a human-readable report by default. For CI, `--format json` and
`--format junit` print a machine-readable one instead. This page is the reference for both, and for the exit
codes they share.

| Command | `text` (default) | `json` | `junit` |
|---|---|---|---|
| `test` | ✓ | ✓ | ✓ |
| `lint` | ✓ | ✓ | ✓ |
| `cover` | ✓ | ✓ | usage error (exit 2) |

- JSON and JUnit go to **stdout**, and never contain colour codes, whatever `NO_COLOR`, `CLICOLOR_FORCE` or the
  terminal say.
- The format never changes the **exit code**.
- **Config, connection and run errors** (a bad spec, an unreachable database) are the same in every format:
  plain text on stderr, **nothing on stdout**, exit 2. There is no JSON error document yet, so a CI step
  that saves stdout to a file gets an empty file in that case.
- The **text** report is for people and changes freely between releases: it cuts descriptions at 64
  characters, shortens UUIDs and puts a long result on the line below its case. Parse JSON or JUnit, never text.

```console
$ rlsspec test --format junit > rlsspec.xml
$ rlsspec lint --format json > lint.json
```

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All good: every case passed, no lint error, no coverage gap under `unspecified: fail` |
| `1` | Test failures, lint errors, or coverage gaps under `unspecified: fail` |
| `2` | Config, connection or run errors, or inconclusive cases (the run is incomplete) |

When a run has both failures and inconclusive cases, `2` wins. Lint warnings and info findings never fail.
A case is *inconclusive* when it couldn't give a verdict: a check that can't fail (an empty table, a predicate
matching no row, or every row for a `rows` write case with `subset`), an SQL error other than a permission
error on the target table (a NOT NULL or foreign key violation, a timeout, a permission error raised by a
trigger or function, such as an audit trigger's own insert), a `rows` write case where a trigger or a cascade
changed other rows of the same table, or a case of a kind this version can't run yet.

A case *skipped* by `on_empty: skip` (a wildcard default on a table with no rows) is none of the above: it
was not run, so it doesn't change the exit code by itself. Its cell counts as unspecified, which is exit 1
under `unspecified: fail`.

## JSON

One pretty-printed document per run. `schema_version` comes first, then `command` (`test`, `cover` or
`lint`), and `exit_code` last. Field names are `snake_case`.

### Stability: `schema_version`

**rlsspec 0.1.0 freezes `schema_version: 1`.** Every field below is stable. Fields added since are marked
with the release that added them; they are always present from that release on:

- A **breaking change** bumps `schema_version`: a field removed, renamed or retyped, or a value whose meaning
  changes.
- **Adding** a field, or a new value where the list is documented as open, is not breaking and keeps the
  version. Consumers should ignore fields they don't know.
- Human-oriented strings (`description`, `detail`, `hint`, `object`) are stable in meaning, not in wording:
  match on `outcome`, `rule` and `severity`, not on text.

`location.file` and `stale_ignore.file` are the spec file path exactly as given with `-c` (default
`rlsspec.yaml`), so they resolve from the working directory rlsspec ran in.

**Order.** Arrays are in report order, which is stable for a given spec and database but not part of the
schema. Cases written under `expect` come in the spec's order; cases a wildcard default expands to, `cover`'s
`tables`, `coverage.gaps` and the JUnit suites follow the tables **by name, then schema** (since 0.2.0; before,
in the order the tables were created). Match on `table`, `identity` and `op`, not on position.

### `test`

```json
{
  "schema_version": 1,
  "command": "test",
  "cases": [
    {
      "table": "organizations",
      "identity": "ana",
      "op": "select",
      "origin": "default",
      "description": "rows: id = '0e5e0000-0000-4000-8000-00000000ac3e'",
      "outcome": "pass",
      "detail": "1 visible",
      "location": { "file": "rlsspec.yaml", "line": 39, "column": 30 }
    }
  ],
  "skipped": [
    {
      "table": "drafts",
      "identity": "anon",
      "op": "select",
      "origin": "default",
      "description": "deny",
      "reason": "empty_table",
      "location": { "file": "rlsspec.yaml", "line": 34, "column": 20 }
    }
  ],
  "coverage": {
    "policy": "fail",
    "total": 80,
    "specified": 75,
    "unspecified": 5,
    "gaps": [
      { "table": "drafts", "identity": "anon", "ops": ["select", "update", "delete"] },
      { "table": "tasks", "identity": "cleo", "ops": ["insert", "update"] }
    ]
  },
  "totals": { "passed": 76, "failed": 0, "inconclusive": 0, "skipped": 3 },
  "exit_code": 1
}
```

| Field | Type | Meaning |
|---|---|---|
| `cases` | array | Every case that ran, in run order, **never folded** (the text report folds `defaults` into one line per identity × table; JSON doesn't) |
| `cases[].table` | string | The table as displayed: `name`, or `schema.name` when the name alone is ambiguous |
| `cases[].identity` | string | Identity name from the spec |
| `cases[].op` | string | `select`, `insert`, `update` or `delete` |
| `cases[].origin` | string | `expect` or `default` |
| `cases[].description` | string | What the case checks, literals in full (`rows: …`, `where … → deny`, `set body = 'x' · rows: …`, `deny`) |
| `cases[].outcome` | string | `pass`, `fail` or `inconclusive`. Closed: a skipped case is listed in `skipped`, never here |
| `cases[].detail` | string | The result (`12 visible`, `leaked 2 of the 4 rows it should not see: id=5, id=6`, `affected 3 of 3 rows (expected 0)`, `updated 1 of the 3 rows it should not update: id=4`, the SQL error) |
| `cases[].location` | object | `{file, line, column}` of the YAML node the case comes from. For `defaults`, the default's entry, shared by every table it expands to. Every case has one |
| `skipped` | array | *Since 0.2.0.* The cases that were not run, in run order; `[]` when there are none. Each has the `table`, `identity`, `op`, `origin`, `description` and `location` of a case, and a `reason` |
| `skipped[].reason` | string | `empty_table`: a wildcard default with `on_empty: skip`, on a table with no rows (open: new reasons add values) |
| `coverage.policy` | string | `ignore`, `warn` or `fail` (the spec's `unspecified`) |
| `coverage.total`, `.specified`, `.unspecified` | integer | Cells (identity × table × operation) |
| `coverage.gaps` | array | One `{table, identity, ops}` per table × identity with unspecified operations, whatever the policy. `todo` cells are gaps, and so are the cells of skipped cases |
| `totals` | object | `passed`, `failed`, `inconclusive`: counts of `cases`; `skipped` (*since 0.2.0*): the length of `skipped` |
| `exit_code` | integer | The process exit code |

### `cover`

```json
{
  "schema_version": 1,
  "command": "cover",
  "identities": ["anon", "ana", "ben", "cleo"],
  "tables": [
    {
      "table": "organizations",
      "cells": [
        { "identity": "anon", "select": true, "insert": true, "update": true, "delete": true }
      ]
    }
  ],
  "coverage": { "policy": "warn", "total": 64, "specified": 64, "unspecified": 0, "gaps": [] },
  "exit_code": 0
}
```

| Field | Type | Meaning |
|---|---|---|
| `identities` | array of strings | In spec order |
| `tables[].table` | string | As in `test` |
| `tables[].cells` | array | One per identity: `identity`, and `select`, `insert`, `update`, `delete` as booleans (`true` = specified) |
| `coverage`, `exit_code` | | As in `test` |

### `lint`

```json
{
  "schema_version": 1,
  "command": "lint",
  "findings": [
    {
      "rule": "RLS004",
      "severity": "warn",
      "object": "notes",
      "role": "app",
      "hint": "permissive UPDATE policy `anyone_edits` has USING (true): it allows every row",
      "table": { "schema": "public", "name": "notes" },
      "function": null,
      "view": null,
      "identities": ["alice"],
      "stale_ignore": null
    }
  ],
  "ignored": 1,
  "skipped": [],
  "totals": { "error": 0, "warn": 1, "info": 0 },
  "exit_code": 0
}
```

| Field | Type | Meaning |
|---|---|---|
| `findings` | array | In the text report's order (by rule) |
| `findings[].rule` | string | `RLS001` … `RLS008` (open: new rules add IDs) |
| `findings[].severity` | string | `error`, `warn` or `info`. Usually fixed per rule; RLS001 is `info` instead of `error` when no identity role holds a privilege on the table |
| `findings[].object` | string | As printed: a table, `schema.function(args)`, `schema.view`, a role (RLS003), or `file:line` for a stale ignore |
| `findings[].role` | string or null | The identity role concerned, when there is one |
| `findings[].hint` | string | One-line explanation and fix |
| `findings[].table`, `.function`, `.view` | object or null | `{schema, name}` of the object, the others null |
| `findings[].identities` | array of strings | The identities concerned: what `lint.ignore` matches on |
| `findings[].stale_ignore` | object or null | `{file, line, column}` of a `lint.ignore` entry that matched nothing, else null |
| `ignored` | integer | Findings matched by `lint.ignore` (counted, not listed) |
| `skipped` | array | `{rule, reason}` for rules skipped on this server (RLS007 on PostgreSQL 14) |
| `totals` | object | `error`, `warn`, `info`: counts of listed findings |
| `exit_code` | integer | `1` when an error finding is left, else `0` |

## JUnit

JUnit XML for CI test reporters (GitHub, GitLab, Jenkins, Buildkite…). Reports carry no timestamps or
durations, so the same run gives the same file. **The verdict always matches the exit code:** there is an
`<error>` if and only if the exit code is 2, otherwise a `<failure>` if and only if it is 1.

### `test`

`<testsuites name="rlsspec test">`, then:

| JUnit | rlsspec |
|---|---|
| `<testsuite name>` | A table, in report order |
| `<testcase name>` | `identity op description`, e.g. `alice update where tenant_id = '…' → deny` |
| `<testcase classname>` | The table |
| passing test case | A passing case |
| `<failure message>` | A failing case. `message` is the detail; the body adds `at file:line:column` |
| `<error message>` | An inconclusive case, same message and location |
| `<skipped message>` | *Since 0.2.0.* A case skipped by `on_empty: skip`; the message is the reason (`empty table`). It counts in `skipped`, not in `failures` or `errors` |
| `<testsuite name="coverage">` | Only under `unspecified: fail` with gaps: one failing test case per table × identity with gaps (`name` = `identity coverage`, `classname` = the table, message `unspecified: insert, update`) |

Every case is its own test case (never folded). Under `unspecified: warn` or `ignore` there is no coverage
suite: gaps don't fail the run there, so they aren't tests. Under `fail`, the cells of skipped cases fail
there like any other gap, so the verdict still matches the exit code.

```xml
<testsuites name="rlsspec test" tests="3" skipped="0" failures="1" errors="1">
    <testsuite name="notes" tests="3" skipped="0" errors="1" failures="1">
        <testcase name="alice select all" classname="notes"/>
        <testcase name="alice update where tenant_id = &apos;…0b&apos; → deny" classname="notes">
            <failure message="affected 3 of 3 rows (expected 0)">affected 3 of 3 rows (expected 0)
at rlsspec.yaml:26:11</failure>
        </testcase>
        <testcase name="alice update where id &gt; 100 → deny" classname="notes">
            <error message="vacuous: the predicate matches no rows, so this check cannot fail (rlsspec.yaml:27:11)">…</error>
        </testcase>
    </testsuite>
</testsuites>
```

### `lint`

`<testsuites name="rlsspec lint">`, then:

| JUnit | rlsspec |
|---|---|
| `<testsuite name>` | A rule, `RLS001` … `RLS008` |
| `<testcase name>` | The finding's object, plus the role when there is one (`notes app`) |
| `<testcase classname>` | The rule |
| `<failure message>` | An `error` finding; the message is the hint |
| passing test case with `<system-out>` | A `warn` or `info` finding (stale ignores included): `severity: hint`. Warnings never fail the run, so they don't fail the test |
| passing `no findings` test case | A rule with no finding, so a clean lint shows eight green tests |
| `<skipped>` test case | A rule skipped on this server, with the reason |

Ignored findings are not listed.

## Using the reports in CI

With the [GitHub Action](../README.md#github-action), set `format` and `output`; the step still writes the file
before it fails, so a reporter with `if: always()` shows the failures. With the binary, redirect stdout and
keep the exit code:

```yaml
- run: rlsspec test --format junit > rlsspec.xml
- uses: mikepenz/action-junit-report@v6
  if: always()
  with:
    report_paths: rlsspec.xml
```
