-- Permissive policies that never let a row in: their check is false.
create policy no_public_inserts on notes for insert with check (false);
create policy frozen on notes for all to app using (false);
