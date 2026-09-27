# rlsspec

CLI (Rust) that checks Postgres Row Level Security against a human-written spec of expected access.

## Hard rules

1. **Never commit to the target database.** One outer transaction, always rolled back; one savepoint per case.
   No code path may issue `COMMIT`.
2. **No vacuous passes.** A case whose target set is empty is a config error, not a pass.
3. **Only `42501` or 0 affected rows means "denied".** Any other SQL error is *inconclusive* (exit 2).
4. **Never interpolate values into SQL.** GUCs, claims, `values` and `set` go through bound parameters;
   identifiers are quoted; `vars` inside SQL fragments are quoted literals. Only user-supplied predicates
   (`rows`, `where`) are raw SQL, by design.
5. **The engine is vendor-neutral.** It only knows role + GUCs. Anything platform-specific (Supabase…) lives
   in `src/preset/` as a config → config transform.
6. **Tests hit a real Postgres** (testcontainers). Don't mock the database.
7. **Every new check ships with a red/green fixture**: a correct policy that passes and a broken one that fails
   with the expected message.
8. **Examples are generic and written from scratch.** Never copy schemas, names or data from employer,
   client or private projects into this repo.

## Conventions

- Rust stable. `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass.
- Synchronous code only: the blocking `postgres` crate, no `async`/tokio in our code.
- Errors: `thiserror` enums in the library, `anyhow` only in `main.rs`. No `unwrap()`/`expect()` outside tests
  unless the invariant is explained in a comment.
- Code, comments, commits and docs in English. Commit subjects like `runner: classify 42501 on insert`.
