grant select on notes to web_anon;
create policy everyone on notes for select to web_anon using (published);
