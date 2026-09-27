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
  ✓ alice   select  rows: tenant_id = '…0a'          12 visible
  ✗ alice   update  where tenant_id = '…0b' → deny    affected 3 of 3 rows (expected 0)
  ✓ guest   *       deny                              4/4 ops
coverage 86/96 cells (89.6%) · 10 unspecified
1 failed · 41 passed · 0 inconclusive
```

## Planned features (v0.1)

- **Intent-based checks** for `SELECT`, `INSERT`, `UPDATE` and `DELETE`, run as each identity against a real
  database. Reports rows that *leaked* and rows that were wrongly *hidden*.
- **Safe by construction**: everything runs in one transaction that is always rolled back; remote hosts are
  refused unless explicitly allowed.
- **No false passes**: a check that can't fail is an error, and only a real permission error counts as "denied".
- **Coverage matrix** of identities × tables × operations, with an option to fail CI when a new table has no spec.
- **Lint** for common RLS foot-guns: RLS disabled or not forced, `BYPASSRLS` roles, `USING (true)`, unsafe
  `SECURITY DEFINER` functions, views that bypass RLS.
- **Vendor-neutral**: an identity is a Postgres role + session settings. A `supabase` preset maps JWT claims
  so `auth.uid()` and friends just work.
- **CI-friendly**: single static binary, stable exit codes, JSON and JUnit output, GitHub Action.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
