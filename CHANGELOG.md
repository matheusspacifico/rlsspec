# Changelog

All notable changes to rlsspec are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and rlsspec adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/matheusspacifico/rlsspec/commits/main
