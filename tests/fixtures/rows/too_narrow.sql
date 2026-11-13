-- Broken the other way: only the author may edit or delete, though the whole tenant should.
drop policy edit on notes;
drop policy remove on notes;
create policy edit on notes for update to app
    using (author_id = current_setting('app.user_id')::uuid);
create policy remove on notes for delete to app
    using (author_id = current_setting('app.user_id')::uuid);
