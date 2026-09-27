create function note_count() returns bigint
    language sql stable security definer
    set search_path = ''
    as $$ select count(*) from public.notes $$;

-- A member of an extension is the extension's to fix (pg_graphql's functions on Supabase): not reported.
create extension citext;
create function citext_probe() returns text
    language sql stable security definer
    as $$ select 'ok' $$;
alter extension citext add function citext_probe();
