-- The correct setup every red fixture breaks one piece of: RLS enabled and forced, no catch-all policy.
alter table notes enable row level security;
alter table notes force row level security;
create policy tenant_read on notes for select to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
create policy tenant_write on notes for all to app
    using (tenant_id = current_setting('app.tenant_id')::uuid)
    with check (tenant_id = current_setting('app.tenant_id')::uuid);
