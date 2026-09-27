create function note_count() returns bigint
    language sql stable security definer
    set search_path = ''
    as $$ select count(*) from public.notes $$;
