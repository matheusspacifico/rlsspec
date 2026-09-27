-- Broken: no TO clause, so this permissive policy applies to PUBLIC, `app` included.
create policy suggest on notes for insert with check (published);
