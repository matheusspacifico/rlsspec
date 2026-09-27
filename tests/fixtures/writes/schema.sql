-- Two tenants (…0a, …0b). Alice (…a11ce) wrote notes 1 and 3; everyone in `app` may read every note,
-- so the write policies alone decide what these fixtures test.
create table notes (
    id int primary key,
    tenant_id uuid not null,
    author_id uuid not null,
    published boolean not null default false,
    body text not null
);

insert into notes (id, tenant_id, author_id, published, body) values
    (1, '00000000-0000-4000-8000-00000000000a', '00000000-0000-4000-8000-0000000a11ce', false, 'draft'),
    (2, '00000000-0000-4000-8000-00000000000a', '00000000-0000-4000-8000-00000000ca01', true,  'welcome'),
    (3, '00000000-0000-4000-8000-00000000000a', '00000000-0000-4000-8000-0000000a11ce', false, 'todo'),
    (4, '00000000-0000-4000-8000-00000000000b', '00000000-0000-4000-8000-000000000b0b', true,  'hello'),
    (5, '00000000-0000-4000-8000-00000000000b', '00000000-0000-4000-8000-000000000b0b', false, 'secret'),
    (6, '00000000-0000-4000-8000-00000000000b', '00000000-0000-4000-8000-000000000b0b', false, 'plans');

alter table notes enable row level security;
alter table notes force row level security;
grant select, insert, update, delete on notes to app;
create policy read on notes for select to app using (true);

create table tags (
    id int primary key,
    tenant_id uuid not null,
    name text not null
);

insert into tags (id, tenant_id, name) values
    (1, '00000000-0000-4000-8000-00000000000a', 'red'),
    (2, '00000000-0000-4000-8000-00000000000b', 'blue');

alter table tags enable row level security;
alter table tags force row level security;
grant select on tags to app;
create policy read on tags for select to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
