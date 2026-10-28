# style: keyword-case=upper
select Count(*), lower(a), MyFunc(a) from t;
=====
SELECT COUNT(*), LOWER(a), MyFunc(a)
FROM t;
