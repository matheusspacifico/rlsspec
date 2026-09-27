create role lint_reader;
grant select on notes to lint_reader;
create policy reader on notes for select to lint_reader
    using (tenant_id = current_setting('app.tenant_id')::uuid);
