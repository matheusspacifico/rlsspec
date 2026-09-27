-- Broken: WITH CHECK (true) lets alice move her rows to tenant b.
create policy edit on notes for update to app
    using (tenant_id = current_setting('app.tenant_id')::uuid)
    with check (true);
