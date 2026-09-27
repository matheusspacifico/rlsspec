create policy own_edits on notes for update to app
    using (tenant_id = current_setting('app.tenant_id')::uuid)
    with check (tenant_id = current_setting('app.tenant_id')::uuid);
create policy anyone_reads on notes for select to app using (true);
