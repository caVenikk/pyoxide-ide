update accounts a set balance = a.balance - 1, updated = now() from transfers t where a.id = t.account_id and t.done returning a.id;
=====
update accounts a
set balance = a.balance - 1, updated = now()
from transfers t
where a.id = t.account_id and t.done
returning a.id;
