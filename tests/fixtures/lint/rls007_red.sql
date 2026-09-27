-- Created by the superuser running setup, so owned by it: reads notes bypassing RLS.
create view notes_all as select * from notes;
grant select on notes_all to app;

-- Owned by the table's owner, who bypasses RLS on it (not forced).
alter table notes owner to owner_login;
alter table notes no force row level security;
create view notes_by_owner as select * from notes;
alter view notes_by_owner owner to owner_login;
grant select on notes_by_owner to app;

create materialized view note_counts as select tenant_id, count(*) from notes group by tenant_id;
grant select on note_counts to app;

-- Not readable by app.
create view notes_private as select * from notes;
