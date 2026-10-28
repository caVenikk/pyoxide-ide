alter table items add column note text, drop column legacy, alter column total set default 0;
=====
alter table items
    add column note text,
    drop column legacy,
    alter column total set default 0;
