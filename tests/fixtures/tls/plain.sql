-- Fails the run if this session is encrypted.
do $$
begin
    if (select ssl from pg_stat_ssl where pid = pg_backend_pid()) then
        raise exception 'this session is encrypted';
    end if;
end $$;
create table notes (id int primary key);
