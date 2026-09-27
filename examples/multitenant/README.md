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
0 failed · 89 passed · 0 inconclusive
$ rlsspec cover
               anon  ana   ben   cleo
organizations  SIUD  SIUD  SIUD  SIUD
memberships    SIUD  SIUD  SIUD  SIUD
projects       SIUD  SIUD  SIUD  SIUD
tasks          SIUD  SIUD  SIUD  SIUD
coverage 64/64 cells (100.0%) · 0 unspecified (warn)
```

`schema.sql` and `policies.sql` are applied when the container starts, like migrations. `seed.sql` is
rlsspec's `setup`: it runs inside rlsspec's transaction and is rolled back with everything else.

## Break it

```console
$ docker compose exec -T db psql -U postgres < broken.sql
$ rlsspec test
...
tasks
  ✗ ben   delete  where org_id = '…ac3e' and created_by <> '…0b0b' → deny                                        affected 2 of 2 rows (expected 0)
coverage 64/64 cells (100.0%) · 0 unspecified (warn)
1 failed · 88 passed · 0 inconclusive
```

The broken delete policy forgot the "own tasks, or admin" condition, so ben can delete ana's tasks.
`docker compose down -v` resets the database.
