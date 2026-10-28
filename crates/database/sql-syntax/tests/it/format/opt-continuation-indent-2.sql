# style: right-margin=60 continuation-indent=2
select * from t where a = f(aaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbb, cccccccccccccccccccc, dddddddddddddddddddd, eeeeeeeeeeeeeeeeeee);
=====
select *
from t
where a = f(aaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbb,
  cccccccccccccccccccc, dddddddddddddddddddd,
  eeeeeeeeeeeeeeeeeee);
