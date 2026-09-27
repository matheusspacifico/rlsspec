-- A table added in a later migration, without RLS.
create table tags (id int primary key, name text not null);
grant select on tags to app;
