-- Broken: a later migration lets `app` update and delete any task. Reads stay tenant-only, so only
-- a statement that reads no column sees it.
create policy maintenance on tasks for update to app using (true) with check (true);
create policy cleanup on tasks for delete to app using (true);
