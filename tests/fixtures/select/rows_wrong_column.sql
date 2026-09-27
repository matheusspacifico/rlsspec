create policy tenant on notes for select to app
    using (author_id = current_setting('app.user_id')::uuid);
