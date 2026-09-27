create function current_org_id() returns uuid
    language sql stable
    as $$ select nullif(current_setting('app.org_id', true), '')::uuid $$;

create function current_app_user_id() returns uuid
    language sql stable
    as $$ select nullif(current_setting('app.user_id', true), '')::uuid $$;

-- SECURITY DEFINER so that reading memberships here isn't itself filtered by memberships' policies.
create function is_org_admin() returns boolean
    language sql stable security definer
    set search_path = pg_catalog, public
    as $$
        select exists (
            select 1 from public.memberships m
            where m.org_id = current_org_id()
              and m.user_id = current_app_user_id()
              and m.role = 'admin'
        )
    $$;

alter table organizations enable row level security;
alter table organizations force row level security;
create policy org_read on organizations for select to app_user
    using (id = current_org_id());
create policy org_rename on organizations for update to app_user
    using (id = current_org_id() and is_org_admin());

alter table memberships enable row level security;
alter table memberships force row level security;
create policy members_read on memberships for select to app_user
    using (org_id = current_org_id());
create policy members_add on memberships for insert to app_user
    with check (org_id = current_org_id() and is_org_admin());
create policy members_remove on memberships for delete to app_user
    using (org_id = current_org_id() and is_org_admin());

alter table projects enable row level security;
alter table projects force row level security;
create policy projects_read on projects for select to app_user
    using (org_id = current_org_id());
create policy projects_public on projects for select to app_anon
    using (is_public);
create policy projects_add on projects for insert to app_user
    with check (org_id = current_org_id());
create policy projects_edit on projects for update to app_user
    using (org_id = current_org_id())
    with check (org_id = current_org_id());
create policy projects_remove on projects for delete to app_user
    using (org_id = current_org_id() and is_org_admin());

alter table tasks enable row level security;
alter table tasks force row level security;
create policy tasks_read on tasks for select to app_user
    using (org_id = current_org_id());
create policy tasks_add on tasks for insert to app_user
    with check (org_id = current_org_id() and created_by = current_app_user_id());
create policy tasks_edit on tasks for update to app_user
    using (org_id = current_org_id())
    with check (org_id = current_org_id());
create policy tasks_remove on tasks for delete to app_user
    using (org_id = current_org_id() and (created_by = current_app_user_id() or is_org_admin()));
