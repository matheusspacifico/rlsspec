create policy add on notes for insert to app
    with check (tenant_id = current_setting('app.tenant_id')::uuid);
create policy edit on notes for update to app
    using (tenant_id = current_setting('app.tenant_id')::uuid)
    with check (tenant_id = current_setting('app.tenant_id')::uuid);
create policy remove on notes for delete to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
