# style: right-margin=40
select aaaaaaaa, bbbbbbbb, cccccccc, dddddddd, eeeeeeee from t where aaaaaaaa = 1 and bbbbbbbb = 2 and cccccccc = 3;
=====
select aaaaaaaa, bbbbbbbb, cccccccc,
       dddddddd, eeeeeeee
from t
where aaaaaaaa = 1 and bbbbbbbb = 2
    and cccccccc = 3;
