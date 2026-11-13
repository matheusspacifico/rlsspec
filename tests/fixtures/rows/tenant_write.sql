-- Correct: alice reads, edits and deletes the notes of her tenant only.
drop policy read on notes;
create policy read on notes for select to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
create policy edit on notes for update to app
    using (tenant_id = current_setting('app.tenant_id')::uuid)
    with check (tenant_id = current_setting('app.tenant_id')::uuid);
create policy remove on notes for delete to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
