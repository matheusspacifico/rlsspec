create policy remove on notes for delete to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
