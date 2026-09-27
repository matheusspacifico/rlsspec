-- A tenant's notes: `app` reads and writes the ones of its tenant (app.tenant_id).
create table notes (
    id int primary key,
    tenant_id uuid not null,
    body text not null
);
grant select, insert, update, delete on notes to app;
