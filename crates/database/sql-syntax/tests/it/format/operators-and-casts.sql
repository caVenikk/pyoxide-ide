select -a, a+b*c, a||b, x::int, cast(y as text), a[1], -1, not b, f(-1) from t;
=====
select -a, a + b * c, a || b, x::int, cast(y as text), a[1], -1, not b, f(-1)
from t;
