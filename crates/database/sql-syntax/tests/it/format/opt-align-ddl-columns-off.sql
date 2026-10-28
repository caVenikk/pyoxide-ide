# style: align-ddl-columns=false
create table t (id int not null, name varchar(10) default 'x');
=====
create table t
(
    id int not null,
    name varchar(10) default 'x'
);
