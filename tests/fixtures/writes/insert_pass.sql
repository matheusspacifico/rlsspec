create policy add on notes for insert to app
    with check (tenant_id = current_setting('app.tenant_id')::uuid);
