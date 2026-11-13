-- Broken: a second permissive policy per command reaches the rows of tenant b.
create policy edit_b on events for update to app
    using (tenant_id = '00000000-0000-4000-8000-00000000000b');
create policy remove_b on events for delete to app
    using (tenant_id = '00000000-0000-4000-8000-00000000000b');
