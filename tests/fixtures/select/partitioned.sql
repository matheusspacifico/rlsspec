-- A table partitioned by tenant, three rows per partition: every partition numbers its own tuples,
-- so the nine rows share three `ctid`s. Alice's policy shows her tenant only.
create table events (
    id int not null,
    tenant_id uuid not null,
    body text not null,
    primary key (tenant_id, id)
) partition by list (tenant_id);
create table events_a partition of events for values in ('00000000-0000-4000-8000-00000000000a');
create table events_b partition of events for values in ('00000000-0000-4000-8000-00000000000b');
create table events_rest partition of events default;

insert into events (id, tenant_id, body) values
    (1, '00000000-0000-4000-8000-00000000000a', 'a1'),
    (2, '00000000-0000-4000-8000-00000000000a', 'a2'),
    (3, '00000000-0000-4000-8000-00000000000a', 'a3'),
    (4, '00000000-0000-4000-8000-00000000000b', 'b1'),
    (5, '00000000-0000-4000-8000-00000000000b', 'b2'),
    (6, '00000000-0000-4000-8000-00000000000b', 'b3'),
    (7, '00000000-0000-4000-8000-00000000000c', 'c1'),
    (8, '00000000-0000-4000-8000-00000000000c', 'c2'),
    (9, '00000000-0000-4000-8000-00000000000c', 'c3');

alter table events enable row level security;
alter table events force row level security;
grant select on events to app;
create policy tenant on events for select to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);

-- The same layout without a primary key, so rows are named by partition and `ctid`.
create table readings (tenant_id uuid not null, value int not null) partition by list (tenant_id);
create table readings_a partition of readings for values in ('00000000-0000-4000-8000-00000000000a');
create table readings_b partition of readings for values in ('00000000-0000-4000-8000-00000000000b');
insert into readings (tenant_id, value) values
    ('00000000-0000-4000-8000-00000000000a', 1),
    ('00000000-0000-4000-8000-00000000000a', 2),
    ('00000000-0000-4000-8000-00000000000b', 3),
    ('00000000-0000-4000-8000-00000000000b', 4);
alter table readings enable row level security;
alter table readings force row level security;
grant select on readings to app;
create policy tenant on readings for select to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
