create table audit_log (id int primary key, entry text not null);
alter table audit_log enable row level security;
alter table audit_log force row level security;
grant insert on audit_log to app;
create policy app_appends on audit_log for insert to app with check (entry <> '');
