-- Broken: the guest role gets a catch-all policy.
grant select, update on notes to web_anon;
create policy guest on notes to web_anon using (true);
