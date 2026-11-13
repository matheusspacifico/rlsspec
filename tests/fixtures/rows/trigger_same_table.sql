-- A trigger that changes another row of the same table: editing note 1 also touches note 4, which
-- belongs to the other tenant. It runs with its owner's rights, so no policy stops it.
create function bump_linked() returns trigger language plpgsql security definer
    set search_path = public, pg_temp as $$
begin
    update notes set published = published where id = 4;
    return null;
end
$$;
create trigger bump_linked after update on notes
    for each row when (new.id = 1) execute function bump_linked();
