-- Broken: USING (true), and no WITH CHECK so it defaults to USING: alice can edit every tenant's rows.
create policy edit on notes for update to app using (true);
