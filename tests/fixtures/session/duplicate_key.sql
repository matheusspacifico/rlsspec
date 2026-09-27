create table notes (id int primary key, tenant_id uuid not null);
insert into notes (id, tenant_id) values (1, '00000000-0000-4000-8000-00000000000a');

/* the same id again */ insert into notes (id, tenant_id)
    values (1, '00000000-0000-4000-8000-00000000000b');
