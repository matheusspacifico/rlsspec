-- Shared todo lists. Applied like a migration through the Supabase CLI: as postgres, whose default
-- privileges grant every new table to anon, authenticated and service_role. Row Level Security decides.
set role postgres;

create table lists (
    id        uuid primary key default gen_random_uuid(),
    owner_id  uuid not null default auth.uid(),
    title     text not null,
    is_public boolean not null default false
);

create table todos (
    id       uuid primary key default gen_random_uuid(),
    list_id  uuid not null references lists on delete cascade,
    owner_id uuid not null default auth.uid(),
    title    text not null,
    done     boolean not null default false
);

-- A list shared with a user lets them read it and edit its todos.
create table shares (
    list_id uuid not null references lists on delete cascade,
    user_id uuid not null,
    primary key (list_id, user_id)
);

alter table lists enable row level security;
alter table todos enable row level security;
alter table shares enable row level security;
