merge into stock s using deliveries d on s.item = d.item when matched then update set s.qty = s.qty + d.qty when not matched then insert (item, qty) values (d.item, d.qty);
=====
merge into stock s
using deliveries d
on s.item = d.item
when matched then
    update set s.qty = s.qty + d.qty
when not matched then
    insert (item, qty)
    values (d.item, d.qty);
