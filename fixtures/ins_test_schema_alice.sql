insert into posts (
    id, parent_id, site_id, title, url, contents
)
values (
   '227546dfbb7414953e49c1378d9975c4',
   null,
   crsql_site_id(),
   "First Post",
   null,
   "First post!"
);

insert into profiles (
    site_id, about
)
values (
    crsql_site_id(),
    "The O.G. metasyntactial protocol practitioner"
);

select crsql_finalize();
