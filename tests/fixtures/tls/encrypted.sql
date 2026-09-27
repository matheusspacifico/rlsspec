-- Fails the run unless this session is encrypted.
do $$
begin
    if not (select ssl from pg_stat_ssl where pid = pg_backend_pid()) then
        raise exception 'this session is not encrypted';
    end if;
end $$;
create table notes (id int primary key);
