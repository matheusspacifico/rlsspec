-- Broken: USING (true) lets alice delete every tenant's rows.
create policy remove on notes for delete to app using (true);
