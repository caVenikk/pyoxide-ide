# style: keyword-case=lower
SELECT A FROM T WHERE X IS NOT NULL ORDER BY A DESC;
=====
select A
from T
where X is not null
order by A desc;
