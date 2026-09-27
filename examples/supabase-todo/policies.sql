set role postgres;

-- Helpers the policies call. SECURITY DEFINER (as postgres, which bypasses RLS) so that the lists
-- policies can look at shares and the other way round without recursing. Kept out of the API schema.
create schema private;
grant usage on schema private to anon, authenticated;

create function private.owns_list(list uuid) returns boolean
language sql stable security definer set search_path = '' as $$
    select exists (select from public.lists where id = list and owner_id = auth.uid())
$$;

create function private.shared_with_me(list uuid) returns boolean
language sql stable security definer set search_path = '' as $$
    select exists (select from public.shares where list_id = list and user_id = auth.uid())
$$;

create function private.can_read_list(list uuid) returns boolean
language sql stable security definer set search_path = '' as $$
    select exists (select from public.lists where id = list and (is_public or owner_id = auth.uid()))
        or private.shared_with_me(list)
$$;

create function private.can_edit_list(list uuid) returns boolean
language sql stable security definer set search_path = '' as $$
    select private.owns_list(list) or private.shared_with_me(list)
$$;

-- lists: public ones are readable by anyone; the owner does everything; a share only reads.
create policy lists_read on lists for select to anon, authenticated
    using (is_public or owner_id = auth.uid() or private.shared_with_me(id));
create policy lists_create on lists for insert to authenticated
    with check (owner_id = auth.uid());
create policy lists_edit on lists for update to authenticated
    using (owner_id = auth.uid())
    with check (owner_id = auth.uid());
create policy lists_remove on lists for delete to authenticated
    using (owner_id = auth.uid());

-- todos: readable with their list; the owner and collaborators add and edit them, in their own
-- name; only the list owner deletes them.
create policy todos_read on todos for select to anon, authenticated
    using (private.can_read_list(list_id));
create policy todos_create on todos for insert to authenticated
    with check (owner_id = auth.uid() and private.can_edit_list(list_id));
create policy todos_edit on todos for update to authenticated
    using (private.can_edit_list(list_id))
    with check (private.can_edit_list(list_id));
create policy todos_remove on todos for delete to authenticated
    using (private.owns_list(list_id));

-- shares: the list owner shares and unshares; a collaborator sees and removes their own share.
create policy shares_read on shares for select to authenticated
    using (user_id = auth.uid() or private.owns_list(list_id));
create policy shares_create on shares for insert to authenticated
    with check (private.owns_list(list_id));
create policy shares_remove on shares for delete to authenticated
    using (user_id = auth.uid() or private.owns_list(list_id));
