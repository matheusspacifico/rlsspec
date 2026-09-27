-- One finding per severity on top of the lint base: tags without RLS (error), anyone_edits (warn),
-- audit_log with no policy (info); comments' RLS004 finding is ignored.
create table tags (id int primary key, name text not null);
grant select on tags to app;

create table comments (id int primary key, note_id int not null, body text not null);
alter table comments enable row level security;
alter table comments force row level security;
grant select, delete on comments to app;
create policy comments_read on comments for select to app using (true);
create policy anyone_deletes on comments for delete to app using (true);

create policy anyone_edits on notes for update to app using (true);

create table audit_log (id int primary key, entry text not null);
alter table audit_log enable row level security;
alter table audit_log force row level security;
grant insert on audit_log to app;
