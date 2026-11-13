-- What a migration tool leaves next to the application's tables: no RLS, readable by everyone.
create table schema_migrations (version bigint primary key, applied_at timestamptz not null default now());
insert into schema_migrations (version) values (1), (2);
grant select on schema_migrations to public;
