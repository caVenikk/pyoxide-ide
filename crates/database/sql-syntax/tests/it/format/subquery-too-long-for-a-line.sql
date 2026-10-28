select * from (select aaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbb, ccccccccccccccccccc from some_table_with_a_name where aaaaaaaaaaaaaaaa > 1 and bbbbbbbbbbbbbbbbbbbbb < 2) s;
=====
select *
from (select aaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbb, ccccccccccccccccccc
      from some_table_with_a_name
      where aaaaaaaaaaaaaaaa > 1
        and bbbbbbbbbbbbbbbbbbbbb < 2) s;
