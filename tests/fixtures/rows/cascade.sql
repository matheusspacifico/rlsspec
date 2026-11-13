-- A self-referencing foreign key: note 4, of the other tenant, is a reply to note 1 and goes with it.
alter table notes add column reply_to int references notes (id) on delete cascade;
update notes set reply_to = 1 where id = 4;
