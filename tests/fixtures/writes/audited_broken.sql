-- Broken: notes accepts every tenant. Only the audit trigger's insert into note_events is rejected,
-- which is not the policy under test: drop the trigger and every write goes through.
create policy add on notes for insert to app with check (true);
create policy edit on notes for update to app using (true) with check (true);
create policy remove on notes for delete to app using (true);
