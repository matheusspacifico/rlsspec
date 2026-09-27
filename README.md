# rlsspec

**Write down who should see what. `rlsspec` proves your Postgres Row Level Security does exactly that.**

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

> **Status: early development.** The checks below work today; there is no release yet, so you build it from
> source. Expect the spec format to change before v0.1.

## Features

Working now:

- **Intent-based checks** for `SELECT`, `INSERT`, `UPDATE` and `DELETE`, run as each identity against a real
  database. Reports rows that *leaked*, rows that were wrongly *hidden*, and partial writes
  (`affected 2 of 3 rows`).
- **Safe by construction**: everything runs in one transaction that is always rolled back; remote hosts are
  refused unless explicitly allowed.
- **No false passes**: a check that can't fail (empty table, predicate matching nothing) is an error, and only a
  real permission error or zero affected rows counts as "denied".
- **Defaults**: `"*": { select: deny, … }` for an identity, overridden per table.
- **Coverage** of identities × tables × operations after every run, with `unspecified: fail` to break CI when
  a new table has no spec; `rlsspec cover` prints the matrix without running anything.
- **`rlsspec init`** scaffolds a spec from the database, every cell marked `todo`.
- **Vendor-neutral**: an identity is a Postgres role + session settings (GUCs), so it fits any RLS design.
- **CI-friendly** exit codes: `0` all good, `1` failures, `2` config/connection errors or inconclusive cases.

Planned for v0.1:

- A **`supabase` preset** that maps JWT claims so `auth.uid()` and friends just work.
- **Lint** for common RLS foot-guns: RLS disabled or not forced, `BYPASSRLS` roles, `USING (true)`, unsafe
  `SECURITY DEFINER` functions, views that bypass RLS.
- JSON and JUnit output, prebuilt binaries, a GitHub Action.

## Getting started

### Requirements

- **Rust** (stable) to build it: install with [rustup](https://rustup.rs).
- **PostgreSQL 14 or later**, reachable from where you run it. Use a local, CI or disposable database (see
  [Safety](#safety)).
- A **privileged connection**: the `database.url` role must be a superuser, have `BYPASSRLS`, or own the
  tables without `FORCE ROW LEVEL SECURITY`, and must be able to `SET ROLE` to every identity's role.
- **Docker**, only to run the example or the project's own tests.

### Install

```console
$ cargo install --git https://github.com/matheusspacifico/rlsspec
$ rlsspec version
```

### Try the example

[`examples/multitenant`](examples/multitenant) is a small project tracker with every identity × table ×
operation specified:

```console
$ git clone https://github.com/matheusspacifico/rlsspec && cd rlsspec/examples/multitenant
$ docker compose up -d --wait
$ export DATABASE_URL=postgres://postgres:postgres@localhost:54329/postgres
$ rlsspec test
```

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

| Form | Passes when |
|---|---|
| `select: deny` | the identity sees no rows |
| `select: all` | it sees every row |
| `select: { rows: "<sql>" }` | it sees exactly the rows where the predicate holds (`subset: true`: at most those) |
| `insert: [{ values: {…}, expect: allow\|deny }]` | the row is inserted / rejected with a permission error |
| `insert: deny` | the catalog proves no insert can succeed (no privilege, or no permissive policy applies) |
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

Commands: `test`, `cover`, `init`, `version`. Useful flags: `-c/--config <file>`, `--allow-remote` (for hosts outside localhost and `safety.allowed_hosts`),
`--no-color`.

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

This software is provided "as is", without warranty of any kind; see the [license](#license).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
