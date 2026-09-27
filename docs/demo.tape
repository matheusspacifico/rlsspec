# The README's demo GIF. From the repository root, with Docker running:
#   cargo build --release && vhs docs/demo.tape
# It resets the multitenant example's database, so every render starts from scratch.

Output docs/demo.gif

Require docker

Set Shell bash
Set FontSize 16
Set Width 1000
Set Height 460
Set Padding 20
Set TypingSpeed 35ms
Set Framerate 20
Set WaitTimeout 120s

Hide
Type "cd examples/multitenant && export PATH=$PWD/../../target/release:$PATH DATABASE_URL=postgres://postgres:postgres@localhost:54329/postgres CLICOLOR_FORCE=1" Enter
Type "docker compose down -v >/dev/null 2>&1; docker compose up -d --wait >/dev/null 2>&1; clear" Enter
Wait
Show

Type "# rlsspec.yaml: every identity × table × operation of a small multi-tenant app" Enter
Sleep 800ms
Type "rlsspec test | tail -n 2" Sleep 400ms Enter
Wait
Sleep 2.5s

Type "# a policy change forgets that members may only delete their own tasks" Enter
Sleep 800ms
Type "docker compose exec -T db psql -q -U postgres < broken.sql" Sleep 400ms Enter
Wait
Sleep 1s
Type "rlsspec test | grep -v ✓" Sleep 400ms Enter
Wait
Sleep 5s

Type "rlsspec test --format junit | head -n 3" Sleep 400ms Enter
Wait
Sleep 5s
