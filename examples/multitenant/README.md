# Example: multi-tenant project tracker

A small plain-Postgres app where users belong to organizations and only ever see and change their own
organization's data. It's fully specified: every identity × table × operation has a case in
[`rlsspec.yaml`](rlsspec.yaml).

| File | What it is |
|---|---|
| [`schema.sql`](schema.sql) | Roles, tables and grants: `organizations`, `memberships`, `projects`, `tasks` |
| [`policies.sql`](policies.sql) | Row Level Security policies, driven by the `app.org_id` and `app.user_id` settings |
| [`seed.sql`](seed.sql) | Fixture rows: organizations Acme and Globex, users ana, ben and cleo |
| [`rlsspec.yaml`](rlsspec.yaml) | The intended access, per identity and table |
| [`rlsspec-0.1.yaml`](rlsspec-0.1.yaml) | The same spec in 0.1 syntax, which CI runs with the latest released binary |
| [`broken.sql`](broken.sql) | A plausible mistake in one policy, to see a failure |

The rules:

- **Anonymous visitors** (`app_anon`) see public projects and nothing else.
- **Members** (`app_user` with `app.org_id` set) read everything in their organization, create and edit
  projects and tasks there, and delete their own tasks.
- **Admins** also rename the organization, invite and remove members, and delete any project or task.
- Nobody moves rows to another organization, or creates tasks in someone else's name.

## Run it

```console
$ docker compose up -d --wait
$ export DATABASE_URL=postgres://postgres:postgres@localhost:54329/postgres
$ rlsspec test
...
coverage 64/64 cells (100.0%) · 0 unspecified (warn)
0 failed · 76 passed · 0 inconclusive
$ rlsspec cover
               anon  ana   ben   cleo
memberships    SIUD  SIUD  SIUD  SIUD
organizations  SIUD  SIUD  SIUD  SIUD
projects       SIUD  SIUD  SIUD  SIUD
tasks          SIUD  SIUD  SIUD  SIUD
coverage 64/64 cells (100.0%) · 0 unspecified (warn)
$ rlsspec lint
0 errors · 0 warnings · 0 info · 0 ignored
```

`rlsspec lint` reads the catalog only: RLS is enabled and forced on every table, `is_org_admin()` pins its
`search_path`, and `app_anon` holds no grant on the tables it's denied.

`schema.sql` and `policies.sql` are applied when the container starts, like migrations. `seed.sql` is
rlsspec's `setup`: it runs inside rlsspec's transaction and is rolled back with everything else.

## Break it

```console
$ docker compose exec -T db psql -U postgres < broken.sql
$ rlsspec test
...
tasks
  ✗ ben   delete  rows: created_by = '…0b0b'
      deleted 2 of the 4 rows it should not delete: id=…0001, id=…0003
coverage 64/64 cells (100.0%) · 0 unspecified (warn)
1 failed · 75 passed · 0 inconclusive
```

The broken delete policy forgot the "own tasks, or admin" condition, so ben can delete ana's tasks.
`docker compose down -v` resets the database.
