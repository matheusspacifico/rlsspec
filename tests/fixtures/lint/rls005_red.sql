create function note_count() returns bigint
    language sql stable security definer
    as $$ select count(*) from public.notes $$;

-- Not executable by app: in a schema it has no USAGE on, or with EXECUTE revoked.
create schema hidden;
create function hidden.purge() returns void
    language sql security definer
    as $$ delete from public.notes $$;
create function purge_all() returns void
    language sql security definer
    as $$ delete from public.notes $$;
revoke execute on function purge_all() from public;
