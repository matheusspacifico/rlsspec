-- A plausible mistake: the delete policy reuses the "can edit" helper, so collaborators can delete
-- todos too, not just the list owner. Apply it on top of policies.sql and `rlsspec test` fails on
-- carol's todos delete.
set role postgres;
drop policy todos_remove on todos;
create policy todos_remove on todos for delete to authenticated
    using (private.can_edit_list(list_id));
