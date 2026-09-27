# Example: shared todo lists on Supabase

A small Supabase app where users keep todo lists, make some public, and share others with
collaborators. Policies use `auth.uid()`, and the spec uses the `supabase` preset: each identity carries
JWT `claims`, which rlsspec turns into the settings PostgREST would set. It's fully specified, with
`unspecified: fail`: every identity × table × operation has a case in [`rlsspec.yaml`](rlsspec.yaml).

| File | What it is |
|---|---|
| [`schema.sql`](schema.sql) | Tables `lists`, `todos` and `shares`, created as `postgres` so Supabase's default grants apply |
| [`policies.sql`](policies.sql) | Row Level Security policies, driven by `auth.uid()` |
| [`seed.sql`](seed.sql) | Fixture rows: users alice, bob and carol, four lists, five todos, one share |
| [`rlsspec.yaml`](rlsspec.yaml) | The intended access, per identity and table |
| [`broken.sql`](broken.sql) | A plausible mistake in one policy, to see a failure |

The rules:

- **Visitors** (`anon`) read public lists and their todos, and nothing else.
- **Owners** (`authenticated`, with their user id as `sub`) do everything with their lists, their todos and
  their shares.
- **Collaborators** read a list shared with them, add and edit its todos in their own name, and can leave the
  list; they can't rename it, delete its todos or share it further.
- Nobody writes in someone else's name, gives a list away, or moves todos into a list they can't edit.

## Run it

It uses the [`supabase/postgres`](https://hub.docker.com/r/supabase/postgres) image, the database the
Supabase CLI runs locally: it ships the `auth` schema, `auth.uid()` and the `anon` and `authenticated` roles.

```console
$ docker compose up -d --wait
$ export DATABASE_URL=postgres://postgres:postgres@localhost:54330/postgres
$ rlsspec test
...
coverage 48/48 cells (100.0%) · 0 unspecified (fail)
0 failed · 67 passed · 0 inconclusive
$ rlsspec cover
        anon  alice  bob   carol
lists   SIUD  SIUD   SIUD  SIUD
todos   SIUD  SIUD   SIUD  SIUD
shares  SIUD  SIUD   SIUD  SIUD
coverage 48/48 cells (100.0%) · 0 unspecified (fail)
$ rlsspec lint
0 errors · 0 warnings · 0 info · 1 ignored
```

The ignored finding is RLS008 on `shares`: `anon` is denied everything there, yet holds privileges on it.
Supabase's default privileges grant every table in `public` to `anon`, as in any real project, and RLS denies
it; `lint.ignore` in the spec says so. (`bob` is denied everything on `shares` too, but his role,
`authenticated`, needs those grants for alice and carol, so that's not a finding.)

`database.url` connects as `postgres`, which in this image has `BYPASSRLS`, owns the tables and can `SET ROLE`
to `anon` and `authenticated`. `schema.sql` and `policies.sql` run when the container starts, like
migrations. `seed.sql` is rlsspec's `setup`: it runs inside rlsspec's transaction and is rolled back with
everything else.

## Break it

```console
$ docker compose exec -T db psql -U postgres < broken.sql
$ rlsspec test
...
todos
  ✗ carol  delete  deny                                                                               affected 2 of 5 rows (expected 0)
...
1 failed · 66 passed · 0 inconclusive
```

The broken delete policy reuses the "can edit" helper, so carol, a collaborator, can delete the todos of
alice's groceries list. `docker compose down -v` resets the database.
