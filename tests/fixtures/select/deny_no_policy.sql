grant select on notes to web_anon;
create policy tenant on notes for select to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
