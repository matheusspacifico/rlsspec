# Changelog

All notable changes to rlsspec are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and rlsspec adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

A blanket spec you can write on a real schema, from rebuilding the specs of two real projects on 0.1.1.
Everything is additive under `version: 1`: a spec that runs on 0.1.x keeps its meaning and its exit code,
with one exception, a bug fix (partitioned tables, below). An older rlsspec rejects the new keys, so upgrade
before using them.

### Added

- **`rows` for `update` and `delete`**: `update: { set: {…}, rows: "<sql>" }` and `delete: { rows: "<sql>" }`
  pass when the statement, run over the whole table, touches exactly the rows the predicate matches
  (`subset: true`: only such rows, maybe fewer). The statement has no `WHERE` and assigns only constants, so
  it reads no column and the write policies alone decide: a `where … → deny` case reads a column, which
  brings the SELECT policies in and can hide a write policy that is too wide. Failures name the rows:
  `updated 2 of the 5 rows it should not update: id=7, id=9`, `did not delete 1 of the 4 rows it should
  delete: id=3`. A `rows` case can stand alone as the operation's value or sit in a case list next to `where`
  cases, in `expect` and in `defaults`. Inconclusive, never a guess: a trigger or a cascade that changed other
  rows of the same table, a predicate matching no row, and with `subset` a predicate matching every row.
- **`on_empty: skip`** on a wildcard default: its select, update and delete cases on a table with no rows are
  skipped instead of inconclusive. A skipped case is not a pass: its cell counts as unspecified, is listed
  with the gaps and fails the run under `unspecified: fail`. Without the key nothing changes, except that the
  message now says how to skip.
- **`"*[column, …]"`** in `defaults`: a default for every table in scope that has all the listed columns, so
  a rule written in terms of `tenant_id` can be stated once. Precedence per operation is `expect`, a named
  default, `"*[…]"`, then `"*"`. A filter that matches no table is an error, and so are two filters of one
  identity that give the same table the same operation.
- **`database.exclude`**: tables left out of scope (no case, no coverage cell, no table lint rule), for
  migration bookkeeping and the like. An entry that matches no table is an error, and so is an `expect` or
  `defaults` entry that names an excluded table. `rlsspec init --exclude a,b` writes it.
- **Top-level `x-*` keys** are accepted and ignored: a home for YAML anchors shared across the spec.
- JSON `test` reports gain a `skipped` array and `totals.skipped` (always present); JUnit marks skipped cases
  with `<skipped message="empty table"/>`. `schema_version` stays 1.
- `rlsspec test` on a spec where nothing is specified yet ends with a hint pointing at `todo` and
  `rlsspec cover`.

### Changed

- **Tables are listed by name, then schema**, instead of in the order they were created, so reports no longer
  depend on the order migrations ran in. This changes the text reports, `init`, and **the order of JSON
  arrays**: `cover`'s `tables`, `coverage.gaps`, the cases a wildcard default expands to within `cases`, and
  the JUnit suites. No field, type or value changes; cases written under `expect` keep the spec's order.
- The text report cuts descriptions at 64 characters (keeping a final `→ allow` or `→ deny`), puts a result
  that would pass 120 columns on the line below its case, one operation per line for a folded default, and
  shortens UUIDs in the samples of a failure. JSON and JUnit keep everything in full.
- A spec may use an anchor as often as it likes: the YAML parser's alias-to-anchor ratio check, a guard
  against alias bombs in untrusted input, rejected specs with 100 aliases and more than 10 per anchor. Its
  other limits stay.

### Fixed

- **`select` on a partitioned table compared rows by `ctid` alone, which repeats across partitions.** An
  identity that saw the rows of one partition could pass `select: all`, and a correct `rows` case could fail
  with leaks that weren't there. Rows are now compared by `(tableoid, ctid)`. A spec that passed only because
  of this now fails, and one that failed only because of it now passes.


## [0.1.1] - 2026-09-27

A correctness and polish release, from running v0.1.0 against two real projects. No new spec key.

### Fixed

- **A write rejected by a trigger no longer counts as denied.** A permission error (`42501`) raised by a nested
  statement, such as an AFTER INSERT audit trigger whose own insert into another RLS table is rejected, passed
  an `insert`, `update` or `delete` case expecting `deny` even when the target's policy let the row through.
  A deny now counts only when the error is about the target table (no error context, and the message names the
  table or its schema); anything else is inconclusive (exit 2) with the Postgres message and the trigger or
  function it came from. A spec that passed only because of such a trigger now exits 2.
- The `insert: deny` shorthand no longer counts a permissive policy whose check is `false`
  (`WITH CHECK (false)`, or `USING (false)` on an `ALL` policy without a check) as letting rows in.
- Lint RLS005 skips functions that belong to an extension, such as `pg_graphql`'s on every Supabase project.
- Folded `defaults` lines show `?` when an op is inconclusive and none failed, and `✗?` when the line has both.

### Changed

- Lint RLS001 is `info` instead of `error` when no identity role holds a privilege on the table (migration
  bookkeeping tables), and its hint names the roles that see every row.
- A failing `setup` statement is reported at `file:line:column`, from the Postgres error position or the start
  of the statement, with its ordinal in the file and the source line.
- Select failures read `leaked 2 of the 4 rows it should not see` and `hidden 1 of the 3 rows it should see`
  (text and the JSON `detail`, which is free text under `schema_version: 1`).

## [0.1.0] - 2026-09-27

The first release: check Postgres Row Level Security against a spec of the access you intend.

### Added

- **The spec file**, `rlsspec.yaml` with `version: 1`, which is stable from this release: identities as a
  Postgres role plus session settings (GUCs), `vars` substituted as quoted SQL literals in predicates and as
  bound parameters elsewhere (`${name}`, `${env:NAME}`), `setup` SQL files, per-identity `defaults` with `"*"`
  for every table, and `expect` per table × identity × operation. Errors point at the YAML line and column.
- **`rlsspec test`** runs every case as its identity against a real database:
  - `select`: `deny`, `all`, `rows: <predicate>` (exactly those rows, reporting leaked and hidden rows with
    sample keys) and `subset: true`;
  - `insert` with `values`, `update` with `where` and `set`, `delete` with `where`, each `allow` or `deny`,
    plus the `update`/`delete` shorthands over every row and a static `insert: deny` proven from grants and
    policies;
  - partial writes (`affected 2 of 3 rows`) are failures; `values` and `set` are cast to the column types.
- **Safe by construction**: one transaction that is always rolled back, a savepoint per case, no code path
  that commits; lock and statement timeouts; `setup` files refused if they contain transaction control;
  predicates sent through the extended protocol; non-local hosts refused unless listed in
  `safety.allowed_hosts` or allowed with `--allow-remote`, which warns.
- **No false passes**: a case that can't fail (an empty table, a predicate matching no row) is inconclusive,
  and only a permission error (`42501`) or zero affected rows counts as denied; any other SQL error is
  inconclusive (exit 2).
- **Coverage** of identities × tables × operations after every run, `todo` cells, and
  `unspecified: ignore|warn|fail` to break CI when a table has no spec; **`rlsspec cover`** prints the matrix
  without running any case.
- **`rlsspec init`** scaffolds a spec from the database, with every cell `todo`.
- **The `supabase` preset**: `claims` on an identity set the JWT settings PostgREST sets, so `auth.uid()`,
  `auth.role()` and `auth.jwt()` see that user; `init --preset supabase` scaffolds `anon` and `authenticated`.
- **`rlsspec lint`** for RLS foot-guns, rules RLS001–RLS008: RLS disabled or not forced, superuser or
  `BYPASSRLS` identity roles, permissive `USING (true)` write policies, `SECURITY DEFINER` functions without a
  pinned `search_path`, views that bypass RLS, tables with no policy, and grants nobody should use; with a
  `lint.ignore` list that requires a reason and reports stale entries.
- **`--format json|junit`** for `test`, `lint` and `cover` (no JUnit for `cover`). The JSON output's
  `schema_version: 1` is frozen from this release; see [docs/output.md](https://github.com/matheusspacifico/rlsspec/blob/main/docs/output.md).
- **TLS** with libpq's `sslmode` (`disable` to `verify-full`) and `sslrootcert`, over rustls with the system
  and Mozilla roots. Non-local hosts must use TLS: `prefer` becomes `require`, and `disable` is refused unless
  `--allow-insecure` is passed, which warns.
- **Exit codes**: `0` all good, `1` test failures, lint errors or gaps under `unspecified: fail`, `2` config,
  connection or inconclusive.
- **Distribution**: static binaries for Linux (x86_64, arm64), macOS (Intel, Apple silicon) and Windows
  (x86_64) with sha256 checksums, shell and PowerShell installers, a Homebrew tap
  (`matheusspacifico/tap/rlsspec`), `cargo install rlsspec`, a Docker image
  (`ghcr.io/matheusspacifico/rlsspec`) and a GitHub Action (`matheusspacifico/rlsspec`).
- Two runnable examples: `examples/multitenant` (plain Postgres) and `examples/supabase-todo`.

[Unreleased]: https://github.com/matheusspacifico/rlsspec/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/matheusspacifico/rlsspec/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/matheusspacifico/rlsspec/releases/tag/v0.1.0
