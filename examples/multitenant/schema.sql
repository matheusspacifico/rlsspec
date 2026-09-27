-- A small multi-tenant project tracker. Users act through `app_user` with two session settings set by
-- the application: `app.org_id` (the organization they're working in) and `app.user_id`.
-- Anonymous visitors act through `app_anon`.

do $$
begin
    if not exists (select from pg_roles where rolname = 'app_user') then
        create role app_user nologin;
    end if;
    if not exists (select from pg_roles where rolname = 'app_anon') then
        create role app_anon nologin;
    end if;
end
$$;

create table organizations (
    id uuid primary key default gen_random_uuid(),
    name text not null
);

create table memberships (
    org_id uuid not null references organizations on delete cascade,
    user_id uuid not null,
    role text not null check (role in ('admin', 'member')),
    primary key (org_id, user_id)
);

create table projects (
    id uuid primary key default gen_random_uuid(),
    org_id uuid not null references organizations on delete cascade,
    name text not null,
    is_public boolean not null default false,
    unique (id, org_id)
);

create table tasks (
    id uuid primary key default gen_random_uuid(),
    org_id uuid not null,
    project_id uuid not null,
    title text not null,
    created_by uuid not null,
    done boolean not null default false,
    foreign key (project_id, org_id) references projects (id, org_id) on delete cascade
);

grant usage on schema public to app_user, app_anon;
grant select, update (name) on organizations to app_user;
grant select, insert, delete on memberships to app_user;
grant select, insert, update, delete on projects to app_user;
grant select, insert, update, delete on tasks to app_user;
grant select on projects to app_anon;
