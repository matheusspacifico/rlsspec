create table drafts (id int primary key, body text not null);
alter table drafts enable row level security;
grant select, delete on drafts to app;
