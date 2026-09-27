-- The UPDATE policy lets alice edit every note, but the SELECT policy only shows her tenant's notes.
drop policy read on notes;
create policy read on notes for select to app
    using (tenant_id = current_setting('app.tenant_id')::uuid);
create policy edit on notes for update to app using (true);
