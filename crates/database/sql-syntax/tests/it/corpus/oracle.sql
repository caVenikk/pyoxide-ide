-- Valid Oracle queries: the Oracle dialect parses them without an error.
-- Blocks are separated by blank lines.

select sysdate from dual

select systimestamp, user, uid from dual

select rownum, e.* from employees e where rownum <= 10

select * from (select e.*, rownum rn from employees e) where rn between 11 and 20

select employee_id, last_name from employees order by salary desc fetch first 5 rows only

select employee_id from employees order by employee_id offset 10 rows fetch next 10 rows only

select employee_id from employees order by salary fetch first 10 percent rows only

select employee_id from employees order by salary fetch first 3 rows with ties

select employee_id from employees offset 5 rows

select level, employee_id, manager_id from employees start with manager_id is null connect by prior employee_id = manager_id

select employee_id from employees connect by prior employee_id = manager_id start with employee_id = 100

select lpad(' ', 2 * level) || last_name from employees start with manager_id is null connect by nocycle prior employee_id = manager_id order siblings by last_name

select connect_by_root last_name as top, sys_connect_by_path(last_name, '/') from employees connect by prior employee_id = manager_id

select level from dual connect by level <= 10

select e.last_name, d.department_name from employees e, departments d where e.department_id = d.department_id(+)

select e.last_name from employees e, departments d where d.department_id(+) = e.department_id and d.location_id(+) = 1700

select city from locations minus select city from warehouses

select city from locations union all select city from warehouses minus select city from closed

select * from (select department_id, job_id, salary from employees) pivot (sum(salary) for department_id in (10 as accounting, 20 as research, 30))

select * from sales pivot (sum(amount) as total, count(*) as cnt for (region, quarter) in (('east', 1) as e1, ('west', 2) as w2)) p

select * from quarterly unpivot (amount for quarter in (q1 as 'Q1', q2 as 'Q2'))

select * from quarterly unpivot include nulls (amount for quarter in (q1, q2))

select nvl(commission_pct, 0), nvl2(commission_pct, 1, 0) from employees

select decode(status, 1, 'new', 2, 'paid', 'other') from orders

select listagg(last_name, ', ') within group (order by last_name) from employees

select department_id, listagg(last_name, ';') within group (order by hire_date) over (partition by department_id) from employees

select max(salary) keep (dense_rank first order by hire_date) from employees

select to_char(hire_date, 'YYYY-MM-DD'), to_date('2024-01-01', 'YYYY-MM-DD') from employees

select trunc(sysdate), add_months(sysdate, 1), months_between(sysdate, hire_date) from employees

select q'[it's]', q'{a;b}', n'unicode' from dual

select date '2024-01-01', timestamp '2024-01-01 10:00:00' from dual

select interval '5' minute, interval '1-2' year to month, interval '3 4:05' day to second from dual

select systimestamp at time zone 'UTC' from dual

select cast(salary as number(10, 2)) from employees

select cast(last_name as varchar2(30)) from employees

select unique department_id from employees

select employee_id from employees where last_name like 'S%' escape '\'

select regexp_substr(email, '[^@]+') from employees

select e.employee_id from employees e where exists (select 1 from job_history j where j.employee_id = e.employee_id)

with dept_costs as (select department_id, sum(salary) total from employees group by department_id) select * from dept_costs where total > (select avg(total) from dept_costs)

with org (id, mgr, lvl) as (select employee_id, manager_id, 1 from employees where manager_id is null union all select e.employee_id, e.manager_id, o.lvl + 1 from employees e join org o on e.manager_id = o.id) select * from org

select e.last_name, d.department_name from employees e join departments d on e.department_id = d.department_id

select e.last_name, d.department_name from employees e left outer join departments d on e.department_id = d.department_id

select * from employees e full outer join departments d on e.department_id = d.department_id

select * from employees natural join departments

select * from departments d cross apply (select * from employees e where e.department_id = d.department_id)

select * from departments d outer apply (select max(salary) m from employees e where e.department_id = d.department_id)

select * from departments d, lateral (select * from employees e where e.department_id = d.department_id)

select employee_id, row_number() over (partition by department_id order by salary desc) rn from employees

select employee_id, sum(salary) over (order by hire_date rows between unbounded preceding and current row) from employees

select employee_id, ratio_to_report(salary) over () from employees

select * from employees as of timestamp (systimestamp - interval '10' minute)

select * from employees sample (10)

select * from sales partition (sales_q1)

select :1, :name from dual

select employee_id from employees for update

select employee_id from employees for update of salary nowait

select employee_id from employees for update wait 5

select employee_id from employees for update skip locked

select "Employee Id" from "Mixed Case Table"

select e."Last" from hr.employees e

select hr.employees.last_name from hr.employees

select count(*), department_id from employees group by department_id having count(*) > 3 order by 1

select department_id, job_id, sum(salary) from employees group by rollup (department_id, job_id)

select department_id, job_id, sum(salary) from employees group by grouping sets ((department_id, job_id), (department_id), ())

select case when salary > 1000 then 'high' else 'low' end grade from employees

select greatest(1, 2), least(3, 4), mod(10, 3), power(2, 3) from dual

select employee_id from employees where hire_date > sysdate - 30 and department_id in (10, 20)

select extract(year from hire_date) from employees

select trim(leading '0' from code) from items

select * from table(my_package.get_rows(10))

select my_schema.my_package.my_function(1) from dual

select last_name from employees order by last_name nulls first

select json_value(doc, '$.a') from docs

select department_id, count(*) from employees group by department_id having count(*) > 1 order by department_id desc

select e.employee_id, e.salary, avg(e.salary) over (partition by e.department_id) avg_sal from employees e

select employee_id, dense_rank() over (order by salary desc) from employees

select employee_id, lag(salary, 1, 0) over (order by hire_date) prev from employees

select first_name || ' ' || last_name full_name from employees

select upper(last_name), initcap(first_name), substr(phone_number, 1, 3) from employees

select instr(email, '@'), length(email), replace(email, '.', '_') from employees

select round(salary / 12, 2), floor(salary), ceil(salary) from employees

select coalesce(commission_pct, 0) * salary from employees

select case department_id when 10 then 'A' when 20 then 'B' else 'C' end from employees

select count(distinct job_id) from employees

select * from employees where department_id in (select department_id from departments where location_id = 1700)

select * from employees e where salary > all (select salary from employees where department_id = 30)

select * from employees e where salary > any (select salary from employees where department_id = 30)

select d.department_name, (select count(*) from employees e where e.department_id = d.department_id) cnt from departments d

select * from employees where hire_date between date '2020-01-01' and date '2020-12-31'

select * from employees where last_name not like 'K%' and manager_id is not null

select * from employees order by department_id, salary desc nulls last

select department_id from employees intersect select department_id from departments

select department_id from employees union select department_id from departments order by 1

select level, lpad('*', level, '*') from dual connect by level <= 5

select employee_id, prior employee_id from employees connect by prior employee_id = manager_id
