delete from log where created < now() - interval '30 days' and level in ('debug', 'trace');
=====
delete from log
where created < now() - interval '30 days' and level in ('debug', 'trace');
