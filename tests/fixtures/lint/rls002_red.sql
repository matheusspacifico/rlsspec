-- The application role owns the table and RLS isn't forced: no policy applies to it.
alter table notes owner to app;
alter table notes no force row level security;
