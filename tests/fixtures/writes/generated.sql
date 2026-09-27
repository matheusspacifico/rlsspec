alter table notes add column body_length int generated always as (length(body)) stored;
