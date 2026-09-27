create policy anyone_edits on notes for update to app using (true);
create policy anyone_deletes on notes for delete using (true);                     -- PUBLIC: applies to app
create policy anyone_reads on notes for select to app using (true);                -- SELECT: not reported
create policy guests_add on notes for insert to web_anon with check (true);        -- not an identity role
create policy capped on notes as restrictive for update to app with check (true);  -- restrictive: harmless
