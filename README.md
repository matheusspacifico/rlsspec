# rlsspec

**Write down who should see what. `rlsspec` proves your Postgres Row Level Security does exactly that.**

![rlsspec test passing, then failing on one delete case after a broken policy, then the JUnit report](https://raw.githubusercontent.com/matheusspacifico/rlsspec/main/docs/demo.gif)

---

Row Level Security moves authorization into the database, where a missing `WHERE` can't leak data.
But policies are code, and they're rarely tested:

- A policy can **exist and still be wrong**: `USING (true)`, a join on the wrong column, a permissive
  policy that silently widens a stricter one. Linters check that policies exist, not that they're correct.
- New tables ship without policies, or with RLS enabled but not forced while the app connects as the owner.

`rlsspec` flips it around: you declare the **access you intend**, and it checks the real database against it.

```yaml
# rlsspec.yaml
identities:
  alice: { role: app, gucs: { app.tenant_id: "${tenant_a}" } }
  guest: { role: web_anon }

defaults:
  guest:
    "*": { select: deny, insert: deny, update: deny, delete: deny }

expect:
  documents:
    alice:
      select: { rows: "tenant_id = ${tenant_a}" }   # exactly these rows: no more, no fewer
      insert:
        - { values: { tenant_id: "${tenant_a}", title: "ok" }, expect: allow }
        - { values: { tenant_id: "${tenant_b}", title: "x" },  expect: deny }
      update:
        - { where: "tenant_id = ${tenant_b}", expect: deny }
      delete: deny
```

```console
$ rlsspec test
documents
  ✓ alice  select  rows: tenant_id = '…000a'                           12 visible
  ✓ alice  insert  values (tenant_id = '…000a', title = 'ok') → allow  inserted
  ✓ alice  insert  values (tenant_id = '…000b', title = 'x') → deny    permission denied
  ✗ alice  update  where tenant_id = '…000b' → deny                    affected 3 of 3 rows (expected 0)
  ✓ alice  delete  deny                                                permission denied
  ✓ guest  *       deny                                                4/4 ops
coverage 8/8 cells (100.0%) · 0 unspecified (warn)
1 failed · 5 passed · 0 inconclusive
```

## Features

- **Intent-based checks** for `SELECT`, `INSERT`, `UPDATE` and `DELETE`, run as each identity against a real
  database. Reports rows that *leaked*, rows that were wrongly *hidden*, and partial writes
  (`affected 2 of 3 rows`).
- **Safe by construction**: everything runs in one transaction that is always rolled back; remote hosts are
  refused unless explicitly allowed.
- **No false passes**: a check that can't fail (empty table, predicate matching nothing) is an error, and only a
  permission error on the table itself or zero affected rows counts as "denied": a write rejected by something
  else, such as an audit trigger's own insert, is inconclusive.
- **Defaults**: `"*": { select: deny, … }` for an identity, overridden per table.
- **Coverage** of identities × tables × operations after every run, with `unspecified: fail` to break CI when
  a new table has no spec; `rlsspec cover` prints the matrix without running anything.
- **`rlsspec init`** scaffolds a spec from the database, every cell marked `todo`.
- **Vendor-neutral**: an identity is a Postgres role + session settings (GUCs), so it fits any RLS design.
- A **`supabase` preset**: give an identity JWT `claims` and `auth.uid()`, `auth.role()` and `auth.jwt()` just
  work (see [Supabase](#supabase)).
- **`rlsspec lint`** for common RLS foot-guns: RLS disabled or not forced, `BYPASSRLS` roles, `USING (true)`,
  unsafe `SECURITY DEFINER` functions, views that bypass RLS, grants nobody should use (see [Lint](#lint)).
- **CI-friendly**: exit codes `0` all good, `1` failures or lint errors, `2` config/connection errors or
  inconclusive cases; `--format json` and `--format junit` reports (see the [output reference](https://github.com/matheusspacifico/rlsspec/blob/main/docs/output.md)); a
  [GitHub Action](#github-action).
- **TLS** with libpq's `sslmode`, required for any non-local host (see [Safety](#safety)).
- **One static binary** for Linux, macOS and Windows: installers, Homebrew, `cargo install`, a Docker image
  (see [Install](#install)).

## Install

**Shell installer** (Linux, macOS):

```console
$ curl --proto '=https' --tlsv1.2 -LsSf https://github.com/matheusspacifico/rlsspec/releases/latest/download/rlsspec-installer.sh | sh
```

**PowerShell installer** (Windows):

```console
PS> powershell -ExecutionPolicy Bypass -c "irm https://github.com/matheusspacifico/rlsspec/releases/latest/download/rlsspec-installer.ps1 | iex"
```

The installers put `rlsspec` in `~/.cargo/bin` (or `$CARGO_HOME/bin`) and add it to your `PATH`. To pin a
release, replace `latest/download` with `download/v0.1.0`.

**Homebrew** (macOS, Linux):

```console
$ brew install matheusspacifico/tap/rlsspec
```

**Cargo** (Rust 1.89 or later):

```console
$ cargo install rlsspec --locked
```

**Archives**: every [release](https://github.com/matheusspacifico/rlsspec/releases) has a `.tar.xz` (Linux x86_64 and arm64, statically linked;
macOS Intel and Apple silicon) or `.zip` (Windows x86_64), each with its `.sha256`.

Then check it runs: `rlsspec version`.

### Docker

```console
$ docker run --rm --network host -v "$PWD":/work -e DATABASE_URL ghcr.io/matheusspacifico/rlsspec:0.1.0 test
```

The image (`linux/amd64`, `linux/arm64`) is distroless with the static binary as its entrypoint, runs as a
non-root user and works in `/work`: mount the directory holding `rlsspec.yaml` and its `setup` files there
(readable by any user). Tags: `0.1.0`, …, and `latest`.

The [safety guard](#safety) sees hosts from inside the container:

- **`--network host` with `localhost`** (Linux; recommended): the container shares the host's network, so
  `localhost` is local and nothing else is needed. The example above works this way.
- **`host.docker.internal`, or a compose service name** (`db`): that's a non-local host. It must be listed in
  `safety.allowed_hosts` (or passed with `--allow-remote`) **and** use TLS, since `sslmode=prefer` becomes
  `require`. A local Postgres without TLS also needs `--allow-insecure`, which prints a warning:

  ```console
  $ docker run --rm -v "$PWD":/work \
      -e DATABASE_URL=postgres://postgres:postgres@host.docker.internal:54329/postgres \
      ghcr.io/matheusspacifico/rlsspec:0.1.0 lint --allow-remote --allow-insecure
  warning: connecting to non-local host `host.docker.internal` (--allow-remote)
  warning: connecting to non-local host `host.docker.internal` without TLS if it doesn't offer it (sslmode=prefer, --allow-insecure)
  0 errors · 0 warnings · 0 info · 0 ignored
  ```

  On Linux without Docker Desktop, add `--add-host=host.docker.internal:host-gateway`. If the error reads
  `Network unreachable`, the name also resolved to an IPv6 address the container can't reach, which hides the
  real reason: add `?hostaddr=<its IPv4 address>` to the URL to see it.

For a server with a private CA, mount the CA and point at it: `?sslmode=verify-full&sslrootcert=/work/ca.pem`,
or `-e SSL_CERT_FILE=/work/ca.pem` to add it to the trusted roots.

### GitHub Action

```yaml
jobs:
  rls:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:17
        env:
          POSTGRES_PASSWORD: postgres
        ports: ["5432:5432"]
        options: --health-cmd pg_isready --health-interval 2s --health-timeout 5s --health-retries 30
    env:
      DATABASE_URL: postgres://postgres:postgres@localhost:5432/postgres
    steps:
      - uses: actions/checkout@v4
      - run: psql "$DATABASE_URL" -f db/schema.sql -f db/policies.sql   # your migrations
      - uses: matheusspacifico/rlsspec@v0.1.0
        with:
          format: junit
          output: rlsspec.xml
      - uses: mikepenz/action-junit-report@v6
        if: always()   # also when rlsspec failed, to show which cases
        with:
          report_paths: rlsspec.xml
```

| Input | Default | |
|---|---|---|
| `version` | the action's tag | Release to run (`0.1.0`). Required when you pin the action to a commit SHA |
| `command` | `test` | `test`, `lint` or `cover` |
| `config` | `rlsspec.yaml` | Spec file |
| `format` | `text` | `text`, `json` or `junit` |
| `output` | | Write the report to this file (text is also shown in the log) |
| `args` | | Extra arguments, e.g. `--allow-remote` |

The action downloads that release's binary for the runner, checks it against the release's sha256 and runs it.
The step fails when rlsspec exits non-zero, after writing `output`; the `exit-code` output has the code. It
runs on Linux and macOS runners, and `DATABASE_URL` comes from your job's `env` (never printed).

A job like the one above reaches its service on `localhost`, which is local. A job that runs in a `container:`
reaches the service by its name (`postgres`), a non-local host: allow it and use TLS, as for
[Docker](#docker).

## Getting started

### Requirements

- **PostgreSQL 14 or later**, reachable from where you run it. Use a local, CI or disposable database (see
  [Safety](#safety)).
- A **privileged connection**: the `database.url` role must be a superuser, have `BYPASSRLS`, or own the
  tables without `FORCE ROW LEVEL SECURITY`, and must be able to `SET ROLE` to every identity's role.
- **Docker**, only to run the example or the project's own tests.

### Try the example

[`examples/multitenant`](https://github.com/matheusspacifico/rlsspec/tree/main/examples/multitenant) is a small project tracker with every identity × table ×
operation specified:

```console
$ git clone https://github.com/matheusspacifico/rlsspec && cd rlsspec/examples/multitenant
$ docker compose up -d --wait
$ export DATABASE_URL=postgres://postgres:postgres@localhost:54329/postgres
$ rlsspec test
```

[`examples/supabase-todo`](https://github.com/matheusspacifico/rlsspec/tree/main/examples/supabase-todo) is the same on Supabase: shared todo lists, policies on
`auth.uid()`, the `supabase/postgres` image.

### Write your own spec

Start from your database: `rlsspec init` writes `rlsspec.yaml` with one identity per role that has privileges
on your tables and every identity × table × operation marked `todo` (it reads `DATABASE_URL`; `--schemas`,
`-o` and `--force` change what it reads and writes). Or write it by hand (`-c path/to/spec.yaml` to use
another path):

```yaml
version: 1

database:
  url: ${env:DATABASE_URL}        # read from the environment
  schemas: [public]               # tables in scope

setup:                            # optional SQL files run first, as the privileged role (rolled back)
  - fixtures.sql

vars:                             # ${name}: a quoted literal in SQL predicates, a raw value elsewhere
  tenant_a: 0191e6a2-0000-7000-8000-00000000000a

identities:                       # a Postgres role + session settings
  alice: { role: app, gucs: { app.tenant_id: "${tenant_a}" } }
  guest: { role: web_anon }

expect:                           # table → identity → operations
  documents:
    alice:
      select: { rows: "tenant_id = ${tenant_a}" }
```

Then run `rlsspec test`. The rows your checks need must exist: put them in the database beforehand or in
`setup` files (a check against an empty table is reported as an error, never a pass).

**`version: 1` is stable.** From v0.1.0, a spec that runs on this release keeps running, with the same
meaning, on later releases. New features only add optional keys; an older rlsspec rejects keys it doesn't
know, so upgrade before using them. A change that would break a spec or change what it checks would come
with a new `version`, and a release that can't read your spec's version says so instead of guessing. Lint may
gain rules, so `rlsspec lint` can report findings it didn't before.

| Form | Passes when |
|---|---|
| `select: deny` | the identity sees no rows |
| `select: all` | it sees every row |
| `select: { rows: "<sql>" }` | it sees exactly the rows where the predicate holds (`subset: true`: at most those) |
| `insert: [{ values: {…}, expect: allow\|deny }]` | the row is inserted / rejected with a permission error |
| `insert: deny` | the catalog proves no insert can succeed (no privilege, or no permissive policy applies; one whose check is `false` doesn't count) |
| `update: [{ where: "<sql>", set?: {…}, expect: allow\|deny }]` | every / none of the rows matching `where` are updated |
| `delete: [{ where: "<sql>", expect: allow\|deny }]` | every / none of the rows matching `where` are deleted |
| `update: allow\|deny`, `delete: allow\|deny` | the same, over every row of the table |
| `select: todo`, `insert: todo`… | nothing runs; the cell is reported as unspecified |

`defaults` takes the same operations keyed by identity, then table or `"*"` for every table; entries under
`expect` win over a named table, which wins over `"*"`.

**Coverage.** Every run ends with `coverage 86/96 cells (89.6%) · 10 unspecified (warn)`: a cell is an
identity × table × operation with at least one case. `unspecified: warn` (the default) also lists the gaps,
`ignore` prints only that line, and `fail` makes any gap exit 1, so a table added without a spec breaks CI.
`rlsspec cover` validates the spec and prints the matrix (`SIUD`, `·` for gaps) without running any case.

**Updates and SELECT policies.** Postgres applies the SELECT policies to an `UPDATE` that reads a column
(in `where`, or in the `SET c = c` rlsspec runs when a case has no `set`), so a too-wide UPDATE policy can
hide behind a strict SELECT policy: rows the identity can't see are never updated. For update denies that
matter, add a case that reads no column, which checks the UPDATE policies alone:
`{ where: "true", set: { title: "x" }, expect: deny }`.

### Supabase

Add `preset: supabase` and give identities `claims`: rlsspec sets them the way PostgREST does for a request
with that JWT, so policies on `auth.uid()`, `auth.role()` and `auth.jwt()` see the user.

```yaml
preset: supabase
identities:
  anon: { role: anon, claims: { role: anon } }
  alice:
    role: authenticated
    claims: { sub: "${alice}", email: alice@example.com }   # role: authenticated is added
```

`claims` becomes `request.jwt.claims` (the whole mapping as JSON, nested values included) plus
`request.jwt.claim.<name>` for each top-level string claim, which older `auth.uid()` versions read. Without a
`role` claim the identity's role is added. Point `database.url` at the `postgres` role (it has `BYPASSRLS` and
can switch to `anon` and `authenticated`), e.g. the Supabase CLI's local database:
`postgres://postgres:postgres@127.0.0.1:54322/postgres`. `rlsspec init --preset supabase` scaffolds `anon`
and `authenticated` instead of one identity per role; `service_role` bypasses RLS and is left out.

### Lint

`rlsspec lint` loads the spec like `test` (including `setup`) and reads the catalog only: no case runs and no
identity is applied. It checks the roles of your identities, so a role no identity uses is never reported.

| Rule | Severity | Finding |
|---|---|---|
| RLS001 | error | A table in scope without RLS enabled; info when no identity's role holds a privilege on it |
| RLS002 | error | RLS enabled but not forced, on a table an identity's role owns (the owner bypasses RLS) |
| RLS003 | error | An identity's role is superuser or has `BYPASSRLS` |
| RLS004 | warn | A permissive `INSERT`/`UPDATE`/`DELETE`/`ALL` policy for an identity's role with `USING (true)` or `WITH CHECK (true)` |
| RLS005 | warn | A `SECURITY DEFINER` function an identity's role can execute, without a pinned `search_path` (functions that belong to an extension are left out) |
| RLS006 | info | RLS enabled with no policy: everything is denied (often intended, sometimes a forgotten migration) |
| RLS007 | warn | A view readable by an identity's role that reads RLS tables as its owner (a superuser, `BYPASSRLS` or the tables' owner) without `security_invoker`, or a readable materialized view over RLS tables (PostgreSQL 15+) |
| RLS008 | warn | An identity denied every operation on a table in the spec, whose role still holds privileges on it (when every identity with that role is denied everything there) |

```console
$ rlsspec lint
✗ RLS001  tags                row level security is not enabled: app sees every row
! RLS004  notes          app  permissive UPDATE policy `anyone_edits` has USING (true): it allows every row
! RLS005  public.leak()  app  SECURITY DEFINER without a pinned search_path: add SET search_path = ''
1 error · 2 warnings · 0 info · 1 ignored
```

It exits `1` when an error is left; warnings and info never fail. To accept a finding, ignore it with a reason:

```yaml
lint:
  ignore:
    - { rule: RLS006, table: audit_log, reason: "written only through a SECURITY DEFINER function" }
    - { rule: RLS008, table: shares, identity: anon, reason: "Supabase grants every public table to anon" }
```

An entry matches every finding of its rule whose keys all match: without `table`, every table. Rules take
`table`, `identity`, `function` or `view` (`name` or `schema.name`) as fits the finding. An entry that matches
nothing is reported as a stale ignore, so ignores can't outlive what they excused.

### Output formats

`test`, `lint` and `cover` take `--format text|json|junit` (`text` is the default; `cover` has no JUnit
report). JSON and JUnit go to stdout without colour, and the exit code is the same whatever the format.
Config and connection errors are always plain text on stderr, with nothing on stdout.

- **JSON**: one document per run, starting with `"schema_version": 1` (frozen since v0.1.0) and ending with `"exit_code"`. `test`
  lists every case (`table`, `identity`, `op`, `origin`, `description`, `outcome`, `detail` and its
  `location` in the spec), the coverage with its gaps, and the totals; `cover` the matrix; `lint` the findings,
  the ignored count and the skipped rules. A breaking change to the format bumps `schema_version`.
- **JUnit**: for CI test reporters. `test` has one suite per table and one test per case (a failure is a
  `<failure>`, an inconclusive case an `<error>`), plus a `coverage` suite under `unspecified: fail`. `lint`
  has one suite per rule: errors fail, warnings and info pass with their hint in the output, as they do for
  the exit code.

```console
$ rlsspec test --format junit > rlsspec.xml
```

Every field, the JUnit mapping and the exit codes are in the [output reference](https://github.com/matheusspacifico/rlsspec/blob/main/docs/output.md).

Commands: `test`, `cover`, `lint`, `init`, `version`. Useful flags: `-c/--config <file>`, `--format`,
`--allow-remote` (for hosts outside localhost and `safety.allowed_hosts`), `--allow-insecure` (a non-local
host without TLS), `--no-color`.

## Safety

`rlsspec` is built to leave your data untouched: every check runs inside a single transaction that is always
rolled back, and it refuses non-local hosts unless you allow them. Still, it runs real queries as a privileged
role, so:

- **Run it against local, CI or disposable databases** (a container, a restored copy, a branch). Don't point it
  at production.
- While it runs, it holds row and table locks and an open transaction. On a busy database that can block other
  sessions.
- Write checks really run `INSERT`, `UPDATE` and `DELETE`, so triggers fire and sequences advance. Some
  effects can't be rolled back: sequence values, `dblink`/foreign-data-wrapper writes, and triggers with
  side effects outside the database.
- `rlsspec.yaml` contains SQL that is executed as written. Only run specs you trust.
- Reports can include primary keys of real rows.

**TLS.** `sslmode` in the connection string works as in `psql`: `disable`, `prefer` (the default: TLS when the
server offers it), `require` (always encrypted, certificate not checked), `verify-ca` (signed by a trusted CA)
and `verify-full` (and issued for that host). Trusted CAs are your system's plus Mozilla's, or the PEM file in
`sslrootcert` for a private CA:

```console
$ export DATABASE_URL="postgres://ci:…@db.staging.internal/app?sslmode=verify-full&sslrootcert=ca.pem"
```

A non-local host (listed in `safety.allowed_hosts` or let through by `--allow-remote`) must use TLS: `prefer`
becomes `require`, and `sslmode=disable` is refused unless you pass `--allow-insecure`. `--allow-remote` and
`--allow-insecure` each print a warning naming the host. `sslmode=allow` and client certificates are not
supported.

This software is provided "as is", without warranty of any kind; see the [license](#license).

## License

Licensed under either of [Apache License, Version 2.0](https://github.com/matheusspacifico/rlsspec/blob/main/LICENSE-APACHE) or [MIT license](https://github.com/matheusspacifico/rlsspec/blob/main/LICENSE-MIT) at your option.
