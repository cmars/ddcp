insert into posts (
    id, parent_id, site_id, title, url, contents
)
values (
   '766387967fe06332487edd0d309cf06e',
   null,
   crsql_site_id(),
   "DDCP - Database-to-Database Copy",
   "https://gitlab.com/cmars232/ddcp",
   "Look at this really cool project I found that replicates databases across Veilid!"
), (
    'ec18bc187f2b747e8a92296dbb50c859',
    '227546dfbb7414953e49c1378d9975c4',
    crsql_site_id(),
    null,
    null,
    "Nice one!"
);


insert into profiles (
    site_id, about
)
values (
    crsql_site_id(),
    "Your ever-reliable protocol-demonstrating sidekick"
);

select crsql_finalize();
