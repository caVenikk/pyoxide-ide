select id, case when status = 'new' then 'brand new customer' when status = 'old' then 'long standing customer' else 'someone we do not know yet' end as kind from customers;
=====
select id,
       case
           when status = 'new' then 'brand new customer'
           when status = 'old' then 'long standing customer'
           else 'someone we do not know yet' end as kind
from customers;
