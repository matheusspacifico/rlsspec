-- Three tenant tables and one global one. `labels` has tenant_id but no rows yet; `plans` has no
-- tenant_id at all, so a rule written in terms of that column can't apply to it.
create table projects (id int primary key, tenant_id uuid not null, name text not null);
create table tasks (id int primary key, tenant_id uuid not null, project_id int not null, title text not null);
create table labels (id int primary key, tenant_id uuid not null, name text not null);
create table plans (id int primary key, name text not null);

insert into projects (id, tenant_id, name) values
    (1, '00000000-0000-4000-8000-00000000000a', 'apollo'),
    (2, '00000000-0000-4000-8000-00000000000b', 'gemini');
insert into tasks (id, tenant_id, project_id, title) values
    (1, '00000000-0000-4000-8000-00000000000a', 1, 'plan'),
    (2, '00000000-0000-4000-8000-00000000000a', 1, 'build'),
    (3, '00000000-0000-4000-8000-00000000000b', 2, 'launch');
insert into plans (id, name) values (1, 'free'), (2, 'team');

do $$
declare t text;
begin
    foreach t in array array['projects', 'tasks', 'labels'] loop
        execute format('alter table %I enable row level security', t);
        execute format('alter table %I force row level security', t);
        execute format('grant select, insert, update, delete on %I to app', t);
        execute format(
            'create policy tenant on %I to app
                 using (tenant_id = current_setting(''app.tenant_id'')::uuid)
                 with check (tenant_id = current_setting(''app.tenant_id'')::uuid)', t);
    end loop;
end $$;

alter table plans enable row level security;
alter table plans force row level security;
grant select on plans to app;
create policy everyone on plans for select to app using (true);
