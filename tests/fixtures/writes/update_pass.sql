create policy edit on notes for update to app
    using (tenant_id = current_setting('app.tenant_id')::uuid)
    with check (tenant_id = current_setting('app.tenant_id')::uuid);
