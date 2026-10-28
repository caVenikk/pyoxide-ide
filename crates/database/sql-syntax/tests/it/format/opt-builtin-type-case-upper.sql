# style: builtin-type-case=upper
create table t (a integer, b varchar(10), c timestamp with time zone, d mytype);
=====
create table t
(
    a INTEGER,
    b VARCHAR(10),
    c TIMESTAMP with TIME ZONE,
    d mytype
);
