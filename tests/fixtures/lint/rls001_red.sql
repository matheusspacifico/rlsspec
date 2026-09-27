-- A table added in a later migration, without RLS.
create table tags (id int primary key, name text not null);
grant select on tags to app;

-- Migration bookkeeping no identity role can reach: info, a later grant would expose it.
create table schema_migrations (version bigint primary key);
