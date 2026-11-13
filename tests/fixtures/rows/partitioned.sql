-- The partitioned table of the select fixtures, with write policies: every partition numbers its
-- own tuples, so rows are told apart by (tableoid, ctid).
grant update, delete on events to app;
create policy edit on events for update to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
create policy remove on events for delete to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
