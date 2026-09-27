-- Two tenants (…0a, …0b); alice is the author of notes 1, 3 and 6, the last one in tenant b.
create table notes (
    id int primary key,
    tenant_id uuid not null,
    author_id uuid not null,
    published boolean not null default false,
    archived boolean not null default false,
    body text not null
);

insert into notes (id, tenant_id, author_id, published, archived, body) values
    (1, '00000000-0000-4000-8000-00000000000a', '00000000-0000-4000-8000-0000000a11ce', false, false, 'draft'),
    (2, '00000000-0000-4000-8000-00000000000a', '00000000-0000-4000-8000-00000000ca01', true,  false, 'welcome'),
    (3, '00000000-0000-4000-8000-00000000000a', '00000000-0000-4000-8000-0000000a11ce', false, true,  'old; commit'),
    (4, '00000000-0000-4000-8000-00000000000b', '00000000-0000-4000-8000-000000000b0b', true,  false, 'hello'),
    (5, '00000000-0000-4000-8000-00000000000b', '00000000-0000-4000-8000-000000000b0b', false, false, 'secret'),
    (6, '00000000-0000-4000-8000-00000000000b', '00000000-0000-4000-8000-0000000a11ce', false, false, 'moved');

-- A SQL-standard body: its inner `;` and `end` must not trip the setup lexer.
create function note_tenant(n notes) returns uuid
    language sql stable
begin atomic
    select n.tenant_id;
end;

alter table notes enable row level security;
alter table notes force row level security;
grant select on notes to app;
