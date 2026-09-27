-- An audit trail: every write to notes logs a row in note_events, which has its own tenant policy. The
-- trigger runs as the writer, so a note written into the other tenant is also rejected by note_events.
create table note_events (
    id bigint generated always as identity primary key,
    tenant_id uuid not null,
    note_id int not null,
    action text not null
);

alter table note_events enable row level security;
alter table note_events force row level security;
grant insert on note_events to app;
create policy tenant_log on note_events for insert to app
    with check (tenant_id = current_setting('app.tenant_id')::uuid);

create function log_note() returns trigger language plpgsql as $$
begin
    if tg_op = 'DELETE' then
        insert into note_events (tenant_id, note_id, action) values (old.tenant_id, old.id, tg_op);
    else
        insert into note_events (tenant_id, note_id, action) values (new.tenant_id, new.id, tg_op);
    end if;
    return null;
end
$$;

create trigger log_note after insert or update or delete on notes
    for each row execute function log_note();
