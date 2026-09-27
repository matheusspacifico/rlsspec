-- A plausible mistake: the delete policy forgets that only a task's author or an org admin may delete it.
-- Apply it on top of policies.sql and `rlsspec test` fails on ben's delete cases.
drop policy tasks_remove on tasks;
create policy tasks_remove on tasks for delete to app_user
    using (org_id = current_org_id());
