# style: keyword-case=upper
select  c.Id, count(*) as N, cast(c.x as Varchar(10)), Upper(c.name), now()  from Customers c  where c.id in (1,2) and not c.x is null order by c.Id desc;
=====
SELECT c.Id, COUNT(*) AS N, CAST(c.x AS Varchar(10)), UPPER(c.name), NOW()
FROM Customers c
WHERE c.id IN (1, 2)
  AND NOT c.x IS NULL
ORDER BY c.Id DESC;
