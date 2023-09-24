create table if not exists posts (
    id text primary key,
    parent_id text,
    site_id text,
    created_at datetime not null default current_timestamp,
    title text,
    url text,
    contents text
);

create table if not exists profiles (
    site_id text primary key,
    created_at datetime not null default current_timestamp,
    updated_at datetime,
    about text,
    picture blob
);

create index if not exists posts_parent_idx on posts (parent_id);
create index if not exists posts_site_idx on posts (site_id);

select crsql_as_crr('posts');
select crsql_as_crr('profiles');

select crsql_finalize();
