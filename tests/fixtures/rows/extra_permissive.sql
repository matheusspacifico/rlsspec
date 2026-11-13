-- Broken: a second permissive policy per command lets `app` write published notes of any tenant.
-- The SELECT policy still hides the other tenant, so a statement that reads a column never
-- reaches those rows.
create policy edit_published on notes for update to app using (published) with check (true);
create policy remove_published on notes for delete to app using (published);
