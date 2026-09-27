-- Broken: WITH CHECK (true) lets alice write into tenant b.
create policy add on notes for insert to app with check (true);
