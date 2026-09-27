create table tags (id int primary key, name text not null);
grant select on tags to app;
alter table tags enable row level security;
alter table tags force row level security;
-- A public read: SELECT policies with USING (true) are not RLS004 findings.
create policy tags_read on tags for select to app using (true);
