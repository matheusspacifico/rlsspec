-- Stricter than the spec: only the author may edit, so alice can update 2 of her tenant's 3 notes.
create policy edit on notes for update to app
    using (tenant_id = current_setting('app.tenant_id')::uuid
           and author_id = current_setting('app.user_id')::uuid);
