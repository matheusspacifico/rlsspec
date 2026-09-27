-- PostgreSQL 15 or later: the view applies its reader's policies.
create view notes_all with (security_invoker = true) as select * from notes;
grant select on notes_all to app;
