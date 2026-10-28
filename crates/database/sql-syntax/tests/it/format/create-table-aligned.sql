create table items (id bigint not null primary key, name varchar(100) not null default 'x', total numeric(12, 2) check (total >= 0), note text, constraint uq unique (name), foreign key (id) references other (id) on delete cascade);
=====
create table items
(
    id    bigint not null primary key,
    name  varchar(100) not null default 'x',
    total numeric(12, 2) check (total >= 0),
    note  text,
    constraint uq unique (name),
    foreign key (id) references other (id) on delete cascade
);
